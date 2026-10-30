//! Intent Kernel：从持久事件推导下一步，而不是由调用方手工拼接控制流。

use chrono::Utc;
use else_protocol::event::ScopeState;
use else_protocol::ids::{AttentionId, IntentId, RunId, ScopeId};
use else_protocol::intent::IntentState;

use crate::{ApiError, Result, Runtime};

/// Kernel 对一次推进请求给出的确定性裁决。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KernelAction {
    Idle,
    WaitForUser {
        requests: Vec<AttentionId>,
    },
    WaitForAttentionWindow {
        requests: Vec<AttentionId>,
    },
    WaitForRun {
        scope_id: ScopeId,
        run_id: RunId,
    },
    Run {
        scope_id: ScopeId,
        generation: u64,
        harness: &'static str,
    },
}

impl Runtime {
    /// 从 Event Ledger 推导当前 Intent 的下一步。
    /// 该函数不执行副作用，可用于预演、调试与未来 daemon 的调度循环。
    pub fn kernel_step(
        &self,
        intent_id: &IntentId,
        harness_hint: Option<&str>,
    ) -> Result<KernelAction> {
        let projection = self.projection(intent_id)?;
        if matches!(
            projection.state,
            Some(IntentState::Completed | IntentState::Failed | IntentState::Cancelled)
        ) {
            return Ok(KernelAction::Idle);
        }

        let actionable = projection.actionable_requests(Utc::now());
        if !actionable.is_empty() {
            return Ok(KernelAction::WaitForUser {
                requests: actionable
                    .into_iter()
                    .map(|request| request.id.clone())
                    .collect(),
            });
        }

        let root_scope = projection
            .root_scope_id
            .clone()
            .ok_or_else(|| ApiError::IntentNotFound(intent_id.clone()))?;
        if let Some(run_id) = self.active_run_id(intent_id, &root_scope)? {
            return Ok(KernelAction::WaitForRun {
                scope_id: root_scope,
                run_id,
            });
        }

        if projection.state == Some(IntentState::NeedsDecision) {
            return Ok(KernelAction::WaitForAttentionWindow {
                requests: projection.open_requests.keys().cloned().collect(),
            });
        }

        let scope = projection
            .scopes
            .get(&root_scope)
            .ok_or_else(|| ApiError::NotFound(format!("scope {root_scope}")))?;
        if matches!(
            scope.state,
            ScopeState::Completed | ScopeState::Failed | ScopeState::Cancelled
        ) {
            return Ok(KernelAction::Idle);
        }

        let spec = projection
            .spec
            .clone()
            .ok_or_else(|| ApiError::IntentNotFound(intent_id.clone()))?;
        let harness = else_harness::select_harness(&spec.outcome, harness_hint);
        Ok(KernelAction::Run {
            scope_id: root_scope.clone(),
            generation: self.next_scope_generation(intent_id, &root_scope)?,
            harness,
        })
    }

    /// 执行一次 Kernel 裁决。Waiting 状态不产生任何事件。
    pub fn advance(
        &self,
        intent_id: &IntentId,
        harness_hint: Option<&str>,
    ) -> Result<else_ledger::IntentProjection> {
        match self.kernel_step(intent_id, harness_hint)? {
            KernelAction::Idle
            | KernelAction::WaitForUser { .. }
            | KernelAction::WaitForAttentionWindow { .. }
            | KernelAction::WaitForRun { .. } => self.projection(intent_id),
            KernelAction::Run { harness, .. } if harness == else_harness::SCRIPTED => {
                self.run(intent_id)
            }
            KernelAction::Run { harness, .. } => {
                let adapter = else_harness::find_adapter(harness)
                    .ok_or_else(|| ApiError::NotFound(format!("harness {harness}")))?;
                self.run_adapter(intent_id, adapter.as_ref())
            }
        }
    }
}
