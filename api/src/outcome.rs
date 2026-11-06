//! 人工 Outcome 确认：Decision 只确认 Manual 标准，不重新执行 Harness。

use else_gateway::{ensure_not_fenced, ExecContext, GatewayError};
use else_ledger::ThreadProjection;
use else_protocol::event::{Actor, EventPayload, ScopeState};
use else_protocol::ids::{RunId, ScopeId, ThreadId};
use else_protocol::thread::ThreadState;

use crate::scope::scope_spec;
use crate::{ApiError, Result, Runtime};

impl Runtime {
    pub(crate) fn confirm_manual_verification(
        &self,
        thread_id: &ThreadId,
        scope_id: &ScopeId,
        generation: u64,
        run_id: RunId,
    ) -> Result<ThreadProjection> {
        let projection = self.projection(thread_id)?;
        let spec = scope_spec(&projection, scope_id)
            .ok_or_else(|| ApiError::ThreadNotFound(thread_id.clone()))?;
        let scope = projection
            .scopes
            .get(scope_id)
            .ok_or_else(|| ApiError::NotFound(format!("scope {scope_id}")))?;
        let subagent = scope.spec.is_some();
        let workspace = if subagent {
            self.scope_workspace(thread_id, scope_id)
        } else {
            self.thread_workspace(&spec)?
        };
        let evidence = else_harness::verify::confirm_manual_outcome(&workspace, &spec.done_when)
            .map_err(GatewayError::Model)?;
        let ctx = ExecContext {
            spec,
            scope_id: scope_id.clone(),
            run_id,
            generation,
            snapshot_id: None,
        };
        ensure_not_fenced(&self.ledger, &ctx)?;
        ctx.record(
            &self.ledger,
            Actor::User,
            EventPayload::OutcomeVerified { evidence },
        )?;
        ctx.record(
            &self.ledger,
            Actor::System,
            EventPayload::ScopeStateChanged {
                state: ScopeState::Completed,
            },
        )?;
        if !subagent {
            ctx.record(
                &self.ledger,
                Actor::System,
                EventPayload::ThreadStateChanged {
                    state: ThreadState::Completed,
                },
            )?;
            self.record_episode_if_terminal(thread_id)?;
        }
        self.projection(thread_id)
    }
}
