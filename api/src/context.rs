//! M2 Context & Memory：Context Compiler、Memory 写入晋升、Intent Revision 联动。
//!
//! 权威状态都在 Ledger；Memory 投影与 Context Snapshot 随时可重放重建。
//! Memory 不是权限来源：记忆只进入快照的 `facts`，授权永远来自 IntentSpec。

use chrono::Utc;
use else_context::{promotion_allowed, MemoryProjection};
use else_protocol::event::{Actor, EventEnvelope, EventPayload, ScopeState};
use else_protocol::ids::{IntentId, MemoryId, ScopeId, SnapshotId};
use else_protocol::intent::{IntentSpec, IntentState};
use else_protocol::memory::{
    ContextSnapshot, MemoryRecord, MemoryScope, MemorySource, MemoryTier, SnapshotAudience,
};

use crate::{ApiError, Result, Runtime};

/// 用户发起的 Intent 修订。目标语义版本 +1，只有用户或其明确授权能产生。
#[derive(Debug, Default)]
pub struct IntentRevision {
    pub outcome: Option<String>,
    pub add_constraints: Vec<String>,
    pub reason: String,
}

/// 不隶属于某个 Intent 的记忆事件（项目/全局纠正、晋升、撤销）的挂载点。
fn system_intent() -> IntentId {
    IntentId::from("else-system")
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
    /// - Planner：完整 Intent 字段 + 召回记忆（Episodic/Semantic）+ 产物引用；
    /// - Executor：Intent 字段 + 本 Intent 的 Working 暂存 + 产物引用；
    /// - Verifier：快照 revision 时点的原始 DoneWhen 与约束——
    ///   不看计划、不看召回记忆，验收标准不可被下游内容改写。
    pub fn compile_snapshot(
        &self,
        intent_id: &IntentId,
        audience: SnapshotAudience,
    ) -> Result<ContextSnapshot> {
        let projection = self.projection(intent_id)?;
        let spec = projection
            .spec
            .clone()
            .ok_or_else(|| ApiError::IntentNotFound(intent_id.clone()))?;
        let (scope_id, generation) = self.compile_scope(&projection, intent_id)?;
        self.compile_snapshot_for(intent_id, &spec, &scope_id, generation, audience)
    }

    /// 按指定 spec/scope 编译快照：Dispatch 为子 Scope 编译任务视图时使用。
    pub(crate) fn compile_snapshot_for(
        &self,
        intent_id: &IntentId,
        spec: &IntentSpec,
        scope_id: &ScopeId,
        generation: u64,
        audience: SnapshotAudience,
    ) -> Result<ContextSnapshot> {
        let projection = self.projection(intent_id)?;
        let memory = self.memory_projection()?;
        let now = Utc::now();
        let artifact_refs = || {
            projection
                .artifacts
                .iter()
                .map(|artifact| artifact.id.clone())
                .collect::<Vec<_>>()
        };
        let (facts, artifact_refs, assumptions) = match audience {
            SnapshotAudience::Planner => (
                memory.recall(
                    spec.project.as_deref(),
                    Some(intent_id),
                    &[MemoryTier::Episodic, MemoryTier::Semantic],
                    now,
                ),
                artifact_refs(),
                spec.assumptions.clone(),
            ),
            SnapshotAudience::Executor => (
                memory
                    .recall(
                        spec.project.as_deref(),
                        Some(intent_id),
                        &[MemoryTier::Working],
                        now,
                    )
                    .into_iter()
                    .filter(|record| matches!(record.scope, MemoryScope::Intent(_)))
                    .collect(),
                artifact_refs(),
                spec.assumptions.clone(),
            ),
            SnapshotAudience::Verifier => (vec![], vec![], vec![]),
        };
        let memory_refs = facts.iter().map(|record| record.id.clone()).collect();
        let snapshot = ContextSnapshot {
            id: SnapshotId::new(),
            intent_id: intent_id.clone(),
            intent_revision: spec.revision,
            event_cursor: self.ledger.cursor()?,
            audience,
            outcome: spec.outcome.clone(),
            done_when: spec.done_when.clone(),
            constraints: spec.constraints.clone(),
            assumptions,
            memory_refs,
            facts,
            artifact_refs,
            created_at: now,
        };
        self.ledger.append(&EventEnvelope::new(
            intent_id.clone(),
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
        projection: &else_ledger::IntentProjection,
        intent_id: &IntentId,
    ) -> Result<(ScopeId, u64)> {
        match self.latest_root_scope(intent_id)? {
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

    /// Intent Revision：目标变更 revision +1 并落 IntentRevised；
    /// 运行中的子 Scope 默认受影响而被取消（scope_only 收敛，复用 Abort 机制），
    /// 声明为无关的除外；根 Scope 不取消，新 revision 下继续执行。
    /// 无关分支的产物留在 CAS 与事件流中，不随取消删除。
    pub fn revise_intent(
        &self,
        intent_id: &IntentId,
        update: IntentRevision,
        unaffected_scopes: &[ScopeId],
    ) -> Result<IntentSpec> {
        let projection = self.projection(intent_id)?;
        if matches!(
            projection.state,
            Some(IntentState::Completed | IntentState::Failed | IntentState::Cancelled)
        ) {
            return Err(ApiError::NotFound(format!(
                "active intent {intent_id} (terminal state)"
            )));
        }
        let spec = projection
            .spec
            .clone()
            .ok_or_else(|| ApiError::IntentNotFound(intent_id.clone()))?;
        let mut new_spec = spec.clone();
        new_spec.revision += 1;
        if let Some(outcome) = &update.outcome {
            new_spec.outcome = outcome.clone();
        }
        new_spec
            .constraints
            .extend(update.add_constraints.iter().cloned());
        let root = self.latest_root_scope(intent_id)?;
        self.ledger.append(&EventEnvelope::new(
            intent_id.clone(),
            root.clone().unwrap_or_else(ScopeId::new),
            1,
            new_spec.revision,
            Actor::User,
            EventPayload::IntentRevised {
                spec: new_spec.clone(),
                reason: update.reason.clone(),
            },
        ))?;

        let projection = self.projection(intent_id)?;
        for (scope_id, scope) in &projection.scopes {
            if scope.state != ScopeState::Running {
                continue;
            }
            if Some(scope_id) == root.as_ref() || unaffected_scopes.contains(scope_id) {
                continue;
            }
            self.ledger.append(&EventEnvelope::new(
                intent_id.clone(),
                scope_id.clone(),
                scope.generation,
                new_spec.revision,
                Actor::System,
                EventPayload::ScopeAbortRequested {
                    reason: format!(
                        "Intent revision {}：目标变更，该分支受影响",
                        new_spec.revision
                    ),
                    scope_only: Some(true),
                },
            ))?;
        }
        // 复用 Abort 收敛机制：传播、逐 Scope 收尾、在途副作用有结论；
        // scope_only 取消不会关闭整个 Intent。
        self.recover_abort(intent_id)?;
        Ok(new_spec)
    }

    /// 提出记忆候选：候选不是权威状态，需晋升后才进入召回。
    pub fn propose_memory(&self, record: MemoryRecord) -> Result<MemoryRecord> {
        self.ledger.append(&EventEnvelope::new(
            system_intent(),
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
            system_intent(),
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
            system_intent(),
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
            EventEnvelope::new(system_intent(), ScopeId::new(), 1, 1, Actor::User, payload)
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
    /// 作用域取 Intent 的项目（无项目则只归本 Intent）。同一 Intent 只记一次。
    pub(crate) fn record_episode_if_terminal(&self, intent_id: &IntentId) -> Result<()> {
        let projection = self.projection(intent_id)?;
        let Some(state) = projection.state else {
            return Ok(());
        };
        if !matches!(
            state,
            IntentState::Completed | IntentState::Failed | IntentState::Cancelled
        ) {
            return Ok(());
        }
        let spec = projection
            .spec
            .clone()
            .ok_or_else(|| ApiError::IntentNotFound(intent_id.clone()))?;
        let memory = self.memory_projection()?;
        let already = memory
            .promoted
            .values()
            .chain(memory.candidates.values())
            .any(|record| {
                record.tier == MemoryTier::Episodic
                    && record.predicate == "intent_concluded"
                    && record.subject == spec.outcome
            });
        if already {
            return Ok(());
        }
        let scope = spec
            .project
            .clone()
            .map(MemoryScope::Project)
            .unwrap_or_else(|| MemoryScope::Intent(intent_id.clone()));
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
            .read_intent(intent_id)?
            .iter()
            .filter(|event| {
                matches!(
                    event.envelope.payload,
                    EventPayload::OutcomeVerified { .. } | EventPayload::IntentStateChanged { .. }
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
            predicate: "intent_concluded".into(),
            value,
            source: MemorySource::ExecutionObserved,
            evidence,
            valid_from: now,
            valid_until: None,
            supersedes: None,
            created_at: now,
        };
        let (scope_id, generation) = self.compile_scope(&projection, intent_id)?;
        let envelope = |payload| {
            EventEnvelope::new(
                intent_id.clone(),
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
