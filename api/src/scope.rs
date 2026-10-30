//! M1-W2：Subagent 委托树与按 Scope 执行。

use std::collections::BTreeSet;

use else_gateway::{ExecContext, GatewayError, ModelGateway, ToolGateway};
use else_ledger::IntentProjection;
use else_protocol::effect::SideEffectKind;
use else_protocol::event::{Actor, EventEnvelope, EventPayload, ScopeState};
use else_protocol::ids::{IntentId, RunId, ScopeId};
use else_protocol::intent::{IntentSpec, IntentState};

use crate::{ApiError, Result, Runtime};

impl Runtime {
    /// 委托一个子 Scope。子级继承目标约束与禁令，自主副作用上限只能降低。
    pub fn spawn_subagent(
        &self,
        intent_id: &IntentId,
        parent_scope: &ScopeId,
        task: &str,
        max_autonomous_effect: SideEffectKind,
    ) -> Result<ScopeId> {
        let projection = self.projection(intent_id)?;
        let parent = projection
            .scopes
            .get(parent_scope)
            .ok_or_else(|| ApiError::NotFound(format!("scope {parent_scope}")))?;
        if parent.state != ScopeState::Running {
            return Err(ApiError::NotFound(format!("running scope {parent_scope}")));
        }
        let parent_spec = scope_spec(&projection, parent_scope)
            .ok_or_else(|| ApiError::IntentNotFound(intent_id.clone()))?;
        let mut child_spec = parent_spec.clone();
        child_spec.outcome = task.to_string();
        child_spec.authority.max_autonomous_effect = max_autonomous_effect;
        if !parent_spec.authority.permits_child(&child_spec.authority) {
            return Err(ApiError::PermissionEscalation(format!(
                "parent max={:?}, child max={:?}",
                parent_spec.authority.max_autonomous_effect, max_autonomous_effect
            )));
        }

        let child = ScopeId::new();
        self.ledger.append(&EventEnvelope::new(
            intent_id.clone(),
            parent_scope.clone(),
            parent.generation,
            parent_spec.revision,
            Actor::System,
            EventPayload::SubagentSpawned {
                child: child.clone(),
                spec: child_spec,
            },
        ))?;
        Ok(child)
    }

    /// 在指定 Scope 中启动或续跑。子 Scope 的完成不会误结束根 Intent。
    pub fn run_scope(&self, intent_id: &IntentId, scope_id: &ScopeId) -> Result<IntentProjection> {
        let projection = self.projection(intent_id)?;
        if matches!(
            projection.state,
            Some(IntentState::Completed | IntentState::Failed | IntentState::Cancelled)
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
        let spec = scope_spec(&projection, scope_id)
            .ok_or_else(|| ApiError::IntentNotFound(intent_id.clone()))?;
        let subagent = scope.spec.is_some();
        // 执行前编译 Planner 快照；本 Run 后续事件都标记该上下文版本。
        let snapshot =
            self.compile_snapshot(intent_id, else_protocol::memory::SnapshotAudience::Planner)?;
        let ctx = ExecContext {
            spec,
            scope_id: scope_id.clone(),
            run_id: RunId::new(),
            generation: scope.generation,
            snapshot_id: Some(snapshot.id),
        };
        let lease = self.admit_run(&ctx)?;
        ctx.record(
            &self.ledger,
            Actor::System,
            EventPayload::RunStarted {
                run_id: ctx.run_id.clone(),
                harness: else_harness::SCRIPTED.into(),
                capabilities: None,
            },
        )?;
        let tools = ToolGateway::new(&self.ledger, self.workspace.clone());
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
        let (success, error) = match result {
            Ok(()) => (true, None),
            Err(GatewayError::NeedsApproval(_)) => (true, None),
            Err(e @ GatewayError::Ledger(else_ledger::LedgerError::CrashInjected)) => {
                return Err(e.into());
            }
            Err(e) => (false, Some(e)),
        };
        ctx.record(
            &self.ledger,
            Actor::System,
            EventPayload::RunFinished {
                run_id: ctx.run_id.clone(),
                success,
            },
        )?;
        self.revoke_lease(&ctx, &lease)?;
        if let Some(error) = error {
            return Err(error.into());
        }
        self.record_episode_if_terminal(intent_id)?;
        self.projection(intent_id)
    }

    /// 最近一次根执行使用的 Scope；Decision 续跑不能换 Scope，否则授权会丢失。
    pub(crate) fn latest_root_scope(&self, intent_id: &IntentId) -> Result<Option<ScopeId>> {
        let events = self.ledger.read_intent(intent_id)?;
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
}

pub(crate) fn scope_spec(projection: &IntentProjection, scope_id: &ScopeId) -> Option<IntentSpec> {
    projection
        .scopes
        .get(scope_id)
        .and_then(|scope| scope.spec.clone())
        .or_else(|| projection.spec.clone())
}
