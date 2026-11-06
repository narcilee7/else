//! M2 Context & Memory：Context Compiler、Memory 写入晋升、Thread Revision 联动。
//!
//! 权威状态都在 Ledger；Memory 投影与 Context Snapshot 随时可重放重建。
//! Memory 不是权限来源：记忆只进入快照的 `facts`，授权永远来自 ThreadSpec。

use chrono::Utc;
use else_context::{promotion_allowed, MemoryProjection};
use else_protocol::event::{Actor, EventEnvelope, EventPayload, ScopeState};
use else_protocol::ids::{MemoryId, ScopeId, SnapshotId, ThreadId};
use else_protocol::memory::{
    ContextSnapshot, ExternalEventContext, MemoryRecord, MemoryScope, MemorySource, MemoryTier,
    SnapshotAudience,
};
use else_protocol::plan::Plan;
use else_protocol::thread::{ThreadSpec, ThreadState};

use crate::{ApiError, Result, Runtime};

/// 用户发起的 Thread 修订。目标语义版本 +1，只有用户或其明确授权能产生。
#[derive(Debug, Default)]
pub struct ThreadRevision {
    pub outcome: Option<String>,
    pub add_constraints: Vec<String>,
    pub reason: String,
}

/// 不隶属于某个 Thread 的记忆事件（项目/全局纠正、晋升、撤销）的挂载点。
fn system_thread() -> ThreadId {
    ThreadId::from("else-system")
}

impl Runtime {
    /// Memory 投影：全量重放，进程内存只是缓存。
    pub fn memory_projection(&self) -> Result<MemoryProjection> {
        let mut projection = MemoryProjection::default();
        self.ledger.replay(&mut projection)?;
        Ok(projection)
    }

    /// Context Compiler：按服务对象生成最小只读快照并落 ContextCompiled 事件。
    ///
    /// - Planner：完整 Thread 字段 + 召回记忆（Episodic/Semantic）+ 产物引用；
    /// - Executor：Thread 字段 + 本 Thread 的 Working 暂存 + 产物引用；
    /// - Verifier：快照 revision 时点的原始 DoneWhen 与约束——
    ///   不看计划、不看召回记忆，验收标准不可被下游内容改写。
    pub fn compile_snapshot(
        &self,
        thread_id: &ThreadId,
        audience: SnapshotAudience,
    ) -> Result<ContextSnapshot> {
        let projection = self.projection(thread_id)?;
        let spec = projection
            .spec
            .clone()
            .ok_or_else(|| ApiError::ThreadNotFound(thread_id.clone()))?;
        let (scope_id, generation) = self.compile_scope(&projection, thread_id)?;
        self.compile_snapshot_for(thread_id, &spec, &scope_id, generation, audience)
    }

    /// 按指定 spec/scope 编译快照：Dispatch 为子 Scope 编译任务视图时使用。
    pub(crate) fn compile_snapshot_for(
        &self,
        thread_id: &ThreadId,
        spec: &ThreadSpec,
        scope_id: &ScopeId,
        generation: u64,
        audience: SnapshotAudience,
    ) -> Result<ContextSnapshot> {
        let projection = self.projection(thread_id)?;
        let memory = self.memory_projection()?;
        let now = Utc::now();
        let artifact_refs = || {
            projection
                .artifacts
                .iter()
                .map(|artifact| artifact.id.clone())
                .collect::<Vec<_>>()
        };
        let artifact_paths = || {
            projection
                .artifacts
                .iter()
                .map(|artifact| self.relative_artifact_locator(&artifact.locator))
                .collect::<Vec<_>>()
        };
        let (facts, artifact_refs, artifact_paths, assumptions) = match audience {
            SnapshotAudience::Planner => (
                memory.recall(
                    spec.project.as_deref(),
                    Some(thread_id),
                    &[MemoryTier::Episodic, MemoryTier::Semantic],
                    now,
                ),
                artifact_refs(),
                artifact_paths(),
                spec.assumptions.clone(),
            ),
            SnapshotAudience::Executor => (
                memory
                    .recall(
                        spec.project.as_deref(),
                        Some(thread_id),
                        &[MemoryTier::Working],
                        now,
                    )
                    .into_iter()
                    .filter(|record| matches!(record.scope, MemoryScope::Thread(_)))
                    .collect(),
                artifact_refs(),
                artifact_paths(),
                spec.assumptions.clone(),
            ),
            SnapshotAudience::Verifier => (vec![], vec![], vec![], vec![]),
        };
        let memory_refs = facts.iter().map(|record| record.id.clone()).collect();
        let last_run_seq = projection
            .scopes
            .get(scope_id)
            .and_then(|scope| scope.last_run_started_seq)
            .unwrap_or(0);
        let external_events = if audience == SnapshotAudience::Verifier {
            vec![]
        } else {
            projection
                .external_events
                .iter()
                .filter(|event| event.seq > last_run_seq)
                .map(|event| ExternalEventContext {
                    event_id: event.event_id.clone(),
                    seq: event.seq,
                    source: event.source.clone(),
                    summary: event.summary.clone(),
                    occurred_at: event.occurred_at,
                })
                .collect()
        };
        let plan = match audience {
            SnapshotAudience::Verifier => None,
            SnapshotAudience::Planner => projection.current_plan().cloned(),
            SnapshotAudience::Executor => projection
                .current_plan()
                .map(|plan| plan_for_scope(plan, scope_id)),
        };
        let snapshot = ContextSnapshot {
            id: SnapshotId::new(),
            thread_id: thread_id.clone(),
            thread_revision: spec.revision,
            event_cursor: self.ledger.cursor()?,
            audience,
            outcome: spec.outcome.clone(),
            done_when: spec.done_when.clone(),
            constraints: spec.constraints.clone(),
            assumptions,
            memory_refs,
            facts,
            artifact_refs,
            artifact_paths,
            external_events,
            plan,
            created_at: now,
        };
        self.ledger.append(&EventEnvelope::new(
            thread_id.clone(),
            scope_id.clone(),
            generation,
            spec.revision,
            Actor::System,
            EventPayload::ContextCompiled {
                snapshot: snapshot.clone(),
            },
        ))?;
        Ok(snapshot)
    }

    fn compile_scope(
        &self,
        projection: &else_ledger::ThreadProjection,
        thread_id: &ThreadId,
    ) -> Result<(ScopeId, u64)> {
        match self.latest_root_scope(thread_id)? {
            Some(scope_id) => {
                let generation = projection
                    .scopes
                    .get(&scope_id)
                    .map(|scope| scope.generation)
                    .unwrap_or(1);
                Ok((scope_id, generation))
            }
            None => Ok((ScopeId::new(), 1)),
        }
    }

    fn relative_artifact_locator(&self, locator: &str) -> String {
        std::path::Path::new(locator)
            .strip_prefix(&self.workspace)
            .map(|path| path.to_string_lossy().to_string())
            .unwrap_or_else(|_| locator.to_string())
    }

    /// Thread Revision：目标变更 revision +1 并落 ThreadRevised；
    /// 运行中的子 Scope 默认受影响而被取消（scope_only 收敛，复用 Abort 机制），
    /// 声明为无关的除外；根 Scope 不取消，新 revision 下继续执行。
    /// 无关分支的产物留在 CAS 与事件流中，不随取消删除。
    pub fn revise_thread(
        &self,
        thread_id: &ThreadId,
        update: ThreadRevision,
        unaffected_scopes: &[ScopeId],
    ) -> Result<ThreadSpec> {
        let projection = self.projection(thread_id)?;
        if matches!(
            projection.state,
            Some(ThreadState::Completed | ThreadState::Failed | ThreadState::Cancelled)
        ) {
            return Err(ApiError::NotFound(format!(
                "active thread {thread_id} (terminal state)"
            )));
        }
        let spec = projection
            .spec
            .clone()
            .ok_or_else(|| ApiError::ThreadNotFound(thread_id.clone()))?;
        let mut new_spec = spec.clone();
        new_spec.revision += 1;
        if let Some(outcome) = &update.outcome {
            new_spec.outcome = outcome.clone();
        }
        new_spec
            .constraints
            .extend(update.add_constraints.iter().cloned());
        let root = self.latest_root_scope(thread_id)?;
        self.ledger.append(&EventEnvelope::new(
            thread_id.clone(),
            root.clone().unwrap_or_else(ScopeId::new),
            1,
            new_spec.revision,
            Actor::User,
            EventPayload::ThreadRevised {
                spec: new_spec.clone(),
                reason: update.reason.clone(),
            },
        ))?;

        let projection = self.projection(thread_id)?;
        for (scope_id, scope) in &projection.scopes {
            if scope.state != ScopeState::Running {
                continue;
            }
            if Some(scope_id) == root.as_ref() || unaffected_scopes.contains(scope_id) {
                continue;
            }
            self.ledger.append(&EventEnvelope::new(
                thread_id.clone(),
                scope_id.clone(),
                scope.generation,
                new_spec.revision,
                Actor::System,
                EventPayload::ScopeAbortRequested {
                    reason: format!(
                        "Thread revision {}：目标变更，该分支受影响",
                        new_spec.revision
                    ),
                    scope_only: Some(true),
                },
            ))?;
        }
        // 复用 Abort 收敛机制：传播、逐 Scope 收尾、在途副作用有结论；
        // scope_only 取消不会关闭整个 Thread。
        self.recover_abort(thread_id)?;
        Ok(new_spec)
    }

    /// 提出记忆候选：候选不是权威状态，需晋升后才进入召回。
    pub fn propose_memory(&self, record: MemoryRecord) -> Result<MemoryRecord> {
        self.ledger.append(&EventEnvelope::new(
            system_thread(),
            ScopeId::new(),
            1,
            1,
            Actor::System,
            EventPayload::MemoryCandidateProposed {
                record: record.clone(),
            },
        ))?;
        Ok(record)
    }

    /// 用户确认晋升候选到指定作用域（缺省保持候选原作用域）。
    /// Semantic 晋升规则（确认或 ≥2 独立证据）在此强制执行。
    pub fn promote_memory(
        &self,
        memory_id: &MemoryId,
        scope: Option<MemoryScope>,
    ) -> Result<MemoryRecord> {
        let memory = self.memory_projection()?;
        let mut record = memory
            .candidates
            .get(memory_id)
            .cloned()
            .ok_or_else(|| ApiError::NotFound(format!("memory candidate {memory_id}")))?;
        if let Some(scope) = scope {
            record.scope = scope;
        }
        // CLI promote 即显式用户确认。
        if !promotion_allowed(&record, true) {
            return Err(ApiError::NotFound(format!(
                "memory {memory_id} not promotable"
            )));
        }
        self.ledger.append(&EventEnvelope::new(
            system_thread(),
            ScopeId::new(),
            1,
            1,
            Actor::User,
            EventPayload::MemoryPromoted {
                memory: record.clone(),
            },
        ))?;
        Ok(record)
    }

    /// 撤销记忆（已晋升或候选），撤销本身也是事件，可重放。
    pub fn revoke_memory(&self, memory_id: &MemoryId, reason: &str) -> Result<()> {
        self.ledger.append(&EventEnvelope::new(
            system_thread(),
            ScopeId::new(),
            1,
            1,
            Actor::User,
            EventPayload::MemoryRevoked {
                memory_id: memory_id.clone(),
                reason: reason.to_string(),
            },
        ))?;
        Ok(())
    }

    /// UserStated 纠正：立即晋升到用户指定的作用域。
    /// 作用域由用户显式命名，系统不会把它泛化到 Global。
    pub fn correct_memory(
        &self,
        scope: MemoryScope,
        subject: &str,
        predicate: &str,
        value: &str,
    ) -> Result<MemoryRecord> {
        let now = Utc::now();
        let record = MemoryRecord {
            id: MemoryId::new(),
            tier: MemoryTier::Semantic,
            scope,
            subject: subject.to_string(),
            predicate: predicate.to_string(),
            value: value.to_string(),
            source: MemorySource::UserStated,
            evidence: vec![],
            valid_from: now,
            valid_until: None,
            supersedes: None,
            created_at: now,
        };
        let envelope = |payload| {
            EventEnvelope::new(system_thread(), ScopeId::new(), 1, 1, Actor::User, payload)
        };
        self.ledger
            .append(&envelope(EventPayload::MemoryCandidateProposed {
                record: record.clone(),
            }))?;
        self.ledger.append(&envelope(EventPayload::MemoryPromoted {
            memory: record.clone(),
        }))?;
        Ok(record)
    }

    /// 终态自动写 Episodic 记忆：结论 + 产物引用 + 失败原因，
    /// 作用域取 Thread 的项目（无项目则只归本 Thread）。同一 Thread 只记一次。
    pub(crate) fn record_episode_if_terminal(&self, thread_id: &ThreadId) -> Result<()> {
        let projection = self.projection(thread_id)?;
        let Some(state) = projection.state else {
            return Ok(());
        };
        if !matches!(
            state,
            ThreadState::Completed | ThreadState::Failed | ThreadState::Cancelled
        ) {
            return Ok(());
        }
        let spec = projection
            .spec
            .clone()
            .ok_or_else(|| ApiError::ThreadNotFound(thread_id.clone()))?;
        let memory = self.memory_projection()?;
        let already = memory
            .promoted
            .values()
            .chain(memory.candidates.values())
            .any(|record| {
                record.tier == MemoryTier::Episodic
                    && record.predicate == "thread_concluded"
                    && record.subject == spec.outcome
            });
        if already {
            return Ok(());
        }
        let scope = spec
            .project
            .clone()
            .map(MemoryScope::Project)
            .unwrap_or_else(|| MemoryScope::Thread(thread_id.clone()));
        let artifacts = projection
            .artifacts
            .iter()
            .map(|artifact| format!("{}({})", artifact.locator, artifact.id))
            .collect::<Vec<_>>()
            .join(", ");
        let value = format!(
            "state={state:?}; revision={}; artifacts=[{artifacts}]",
            spec.revision
        );
        let evidence = self
            .ledger
            .read_thread(thread_id)?
            .iter()
            .filter(|event| {
                matches!(
                    event.envelope.payload,
                    EventPayload::OutcomeVerified { .. } | EventPayload::ThreadStateChanged { .. }
                )
            })
            .map(|event| event.envelope.event_id.clone())
            .collect();
        let now = Utc::now();
        let record = MemoryRecord {
            id: MemoryId::new(),
            tier: MemoryTier::Episodic,
            scope,
            subject: spec.outcome.clone(),
            predicate: "thread_concluded".into(),
            value,
            source: MemorySource::ExecutionObserved,
            evidence,
            valid_from: now,
            valid_until: None,
            supersedes: None,
            created_at: now,
        };
        let (scope_id, generation) = self.compile_scope(&projection, thread_id)?;
        let envelope = |payload| {
            EventEnvelope::new(
                thread_id.clone(),
                scope_id.clone(),
                generation,
                spec.revision,
                Actor::System,
                payload,
            )
        };
        self.ledger
            .append(&envelope(EventPayload::MemoryCandidateProposed {
                record: record.clone(),
            }))?;
        self.ledger
            .append(&envelope(EventPayload::MemoryPromoted { memory: record }))?;
        Ok(())
    }
}

fn plan_for_scope(plan: &Plan, scope_id: &ScopeId) -> Plan {
    let mut scoped = plan.clone();
    scoped
        .steps
        .retain(|step| step.scope_id.as_ref().is_none_or(|scope| scope == scope_id));
    scoped
}
