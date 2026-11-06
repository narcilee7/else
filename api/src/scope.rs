//! M1-W2：Subagent 委托树与按 Scope 执行。

use std::collections::BTreeSet;

use else_gateway::{ExecContext, ModelGateway, ToolGateway};
use else_ledger::ThreadProjection;
use else_protocol::effect::SideEffectKind;
use else_protocol::event::{Actor, EventEnvelope, EventPayload, ScopeState};
use else_protocol::ids::{RunId, ScopeId, ThreadId};
use else_protocol::thread::{ThreadSpec, ThreadState};

use crate::{ApiError, Result, Runtime};

impl Runtime {
    /// 委托一个子 Scope。子级继承目标约束与禁令，自主副作用上限只能降低。
    pub fn spawn_subagent(
        &self,
        thread_id: &ThreadId,
        parent_scope: &ScopeId,
        task: &str,
        max_autonomous_effect: SideEffectKind,
    ) -> Result<ScopeId> {
        self.spawn_subagent_with_dependencies(
            thread_id,
            parent_scope,
            task,
            max_autonomous_effect,
            vec![],
        )
    }

    pub fn spawn_subagent_with_dependencies(
        &self,
        thread_id: &ThreadId,
        parent_scope: &ScopeId,
        task: &str,
        max_autonomous_effect: SideEffectKind,
        depends_on: Vec<ScopeId>,
    ) -> Result<ScopeId> {
        let projection = self.projection(thread_id)?;
        let parent = projection
            .scopes
            .get(parent_scope)
            .ok_or_else(|| ApiError::NotFound(format!("scope {parent_scope}")))?;
        if parent.state != ScopeState::Running {
            return Err(ApiError::NotFound(format!("running scope {parent_scope}")));
        }
        let parent_spec = scope_spec(&projection, parent_scope)
            .ok_or_else(|| ApiError::ThreadNotFound(thread_id.clone()))?;
        let mut child_spec = parent_spec.clone();
        child_spec.outcome = task.to_string();
        child_spec.authority.max_autonomous_effect = max_autonomous_effect;
        if !parent_spec.authority.permits_child(&child_spec.authority) {
            return Err(ApiError::PermissionEscalation(format!(
                "parent max={:?}, child max={:?}",
                parent_spec.authority.max_autonomous_effect, max_autonomous_effect
            )));
        }
        for dependency in &depends_on {
            if !projection.scopes.contains_key(dependency) {
                return Err(ApiError::NotFound(format!("dependency scope {dependency}")));
            }
        }

        let child = ScopeId::new();
        self.ledger.append(&EventEnvelope::new(
            thread_id.clone(),
            parent_scope.clone(),
            parent.generation,
            parent_spec.revision,
            Actor::System,
            EventPayload::SubagentSpawned {
                child: child.clone(),
                spec: child_spec,
                depends_on,
            },
        ))?;
        Ok(child)
    }

    /// 在指定 Scope 中启动或续跑。子 Scope 的完成不会误结束根 Thread。
    pub fn run_scope(&self, thread_id: &ThreadId, scope_id: &ScopeId) -> Result<ThreadProjection> {
        self.run_scope_inner(thread_id, scope_id, None)
    }

    pub(crate) fn run_scope_at_generation(
        &self,
        thread_id: &ThreadId,
        scope_id: &ScopeId,
        generation: u64,
    ) -> Result<ThreadProjection> {
        self.run_scope_inner(thread_id, scope_id, Some(generation))
    }

    fn run_scope_inner(
        &self,
        thread_id: &ThreadId,
        scope_id: &ScopeId,
        generation: Option<u64>,
    ) -> Result<ThreadProjection> {
        let projection = self.projection(thread_id)?;
        if matches!(
            projection.state,
            Some(ThreadState::Completed | ThreadState::Failed | ThreadState::Cancelled)
        ) {
            return Ok(projection);
        }
        let scope = projection
            .scopes
            .get(scope_id)
            .ok_or_else(|| ApiError::NotFound(format!("scope {scope_id}")))?;
        if scope.state != ScopeState::Running {
            return Ok(projection);
        }
        self.ensure_slot_available()?;
        self.ensure_scope_not_active(thread_id, scope_id)?;
        let spec = scope_spec(&projection, scope_id)
            .ok_or_else(|| ApiError::ThreadNotFound(thread_id.clone()))?;
        let subagent = scope.spec.is_some();
        let generation = generation.unwrap_or(scope.generation);
        // 根 Scope 在 Thread 的执行位置；子 Scope 在 Runtime 下的隔离目录。
        let workspace = if subagent {
            self.scope_workspace(thread_id, scope_id)
        } else {
            self.thread_workspace(&spec)?
        };
        std::fs::create_dir_all(&workspace)?;
        // 执行前编译 Planner 快照；本 Run 后续事件都标记该上下文版本。
        let snapshot =
            self.compile_snapshot(thread_id, else_protocol::memory::SnapshotAudience::Planner)?;
        let ctx = ExecContext {
            spec,
            scope_id: scope_id.clone(),
            run_id: RunId::new(),
            generation,
            snapshot_id: Some(snapshot.id),
        };
        let lease = self.admit_run_with_failure(&ctx)?;
        ctx.record(
            &self.ledger,
            Actor::System,
            EventPayload::RunStarted {
                run_id: ctx.run_id.clone(),
                harness: else_harness::SCRIPTED.into(),
                capabilities: None,
            },
        )?;
        let sandbox_started = self.start_sandbox(&ctx, &lease)?;
        let tools = ToolGateway::new(&self.ledger, workspace.clone());
        let model = ModelGateway::new(&self.ledger, self.provider()?);
        let result = else_harness::run(
            &ctx,
            &self.ledger,
            &tools,
            &model,
            &self.model_name(),
            else_harness::RunOptions {
                subagent,
                ..Default::default()
            },
        );
        self.finish_run(thread_id, &ctx, &lease, sandbox_started, result)
    }

    /// 最近一次根执行使用的 Scope；Decision 续跑不能换 Scope，否则授权会丢失。
    pub(crate) fn latest_root_scope(&self, thread_id: &ThreadId) -> Result<Option<ScopeId>> {
        if let Some(root) = self.projection(thread_id)?.root_scope_id {
            return Ok(Some(root));
        }
        let events = self.ledger.read_thread(thread_id)?;
        let children: BTreeSet<_> = events
            .iter()
            .filter_map(|event| match &event.envelope.payload {
                EventPayload::ScopeSpawned { child }
                | EventPayload::SubagentSpawned { child, .. } => Some(child.clone()),
                _ => None,
            })
            .collect();
        Ok(events.iter().rev().find_map(|event| {
            if matches!(event.envelope.payload, EventPayload::RunStarted { .. })
                && !children.contains(&event.envelope.scope_id)
            {
                Some(event.envelope.scope_id.clone())
            } else {
                None
            }
        }))
    }

    /// 新建 Run 的 Generation：同一 Scope 每次完整 Run 结束后递增。
    pub(crate) fn next_scope_generation(
        &self,
        thread_id: &ThreadId,
        scope_id: &ScopeId,
    ) -> Result<u64> {
        let events = self.ledger.read_thread(thread_id)?;
        let latest_generation = events
            .iter()
            .filter(|event| {
                event.envelope.scope_id == *scope_id
                    && matches!(
                        event.envelope.payload,
                        EventPayload::RunStarted { .. } | EventPayload::ScopeRetryApproved
                    )
            })
            .map(|event| event.envelope.generation)
            .max();
        if let Some(generation) = latest_generation {
            return Ok(generation.saturating_add(1));
        }
        Ok(self
            .projection(thread_id)?
            .scopes
            .get(scope_id)
            .map(|scope| scope.generation)
            .unwrap_or(1))
    }

    /// 一个 Scope 同时最多有一个在途 Run。
    pub(crate) fn ensure_scope_not_active(
        &self,
        thread_id: &ThreadId,
        scope_id: &ScopeId,
    ) -> Result<()> {
        if let Some(run_id) = self.active_run_id(thread_id, scope_id)? {
            Err(ApiError::Conflict(format!(
                "scope {scope_id} already has an active run {run_id}"
            )))
        } else {
            Ok(())
        }
    }

    pub(crate) fn active_run_id(
        &self,
        thread_id: &ThreadId,
        scope_id: &ScopeId,
    ) -> Result<Option<RunId>> {
        let mut active = BTreeSet::new();
        for event in self.ledger.read_thread(thread_id)? {
            if event.envelope.scope_id != *scope_id {
                continue;
            }
            match event.envelope.payload {
                EventPayload::RunStarted { run_id, .. } => {
                    active.insert(run_id);
                }
                EventPayload::RunFinished { run_id, .. } => {
                    active.remove(&run_id);
                }
                _ => {}
            }
        }
        Ok(active.into_iter().next())
    }
}

pub(crate) fn scope_spec(projection: &ThreadProjection, scope_id: &ScopeId) -> Option<ThreadSpec> {
    projection
        .scopes
        .get(scope_id)
        .and_then(|scope| scope.spec.clone())
        .or_else(|| projection.spec.clone())
}
