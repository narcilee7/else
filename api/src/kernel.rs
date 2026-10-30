//! Intent Kernel：从持久事件推导下一步，并连续推进到等待态。

use chrono::{DateTime, Utc};
use else_ledger::IntentProjection;
use else_protocol::event::ScopeState;
use else_protocol::ids::{AttentionId, IntentId, RunId, ScopeId, TimerId};
use else_protocol::intent::IntentState;

use crate::scope::scope_spec;
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
    WaitForTimer {
        due_at: DateTime<Utc>,
    },
    FireTimer {
        timer_id: TimerId,
    },
    Run {
        scope_id: ScopeId,
        generation: u64,
        harness: &'static str,
    },
}

/// 连续推进停止的原因。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KernelDriveStop {
    Idle,
    WaitForUser,
    WaitForAttentionWindow,
    WaitForRun,
    WaitForTimer,
    StepLimit,
}

#[derive(Debug, Clone, PartialEq)]
pub struct KernelDrive {
    pub projection: IntentProjection,
    pub stopped: KernelDriveStop,
    pub steps: u64,
}

impl Runtime {
    /// 从 Event Ledger 推导当前 Intent 的下一步。
    /// 该函数不执行副作用，可用于预演、调试与调度循环。
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
        let now = Utc::now();
        if let Some(timer) = projection.due_timers(now).into_iter().next() {
            return Ok(KernelAction::FireTimer {
                timer_id: timer.id.clone(),
            });
        }

        let actionable = projection.actionable_requests(now);
        if actionable.iter().any(|request| request.blocking) {
            return Ok(KernelAction::WaitForUser {
                requests: actionable
                    .into_iter()
                    .map(|request| request.id.clone())
                    .collect(),
            });
        }

        let mut active_run = None;
        let mut ready = Vec::new();
        for (index, scope_id) in self
            .scope_scheduling_order(&projection)
            .into_iter()
            .enumerate()
        {
            let Some(scope) = projection.scopes.get(&scope_id) else {
                continue;
            };
            if matches!(
                scope.state,
                ScopeState::Completed | ScopeState::Failed | ScopeState::Cancelled
            ) {
                continue;
            }
            if let Some(run_id) = self.active_run_id(intent_id, &scope_id)? {
                active_run.get_or_insert((scope_id.clone(), run_id));
                continue;
            }
            ready.push((scope.last_run_started_seq.unwrap_or(0), index, scope_id));
        }
        ready.sort_by_key(|(last_started, order, _)| (*last_started, *order));
        if let Some((_, _, scope_id)) = ready.into_iter().next() {
            let spec = scope_spec(&projection, &scope_id)
                .ok_or_else(|| ApiError::NotFound(format!("scope {scope_id}")))?;
            return Ok(KernelAction::Run {
                scope_id: scope_id.clone(),
                generation: self.next_scope_generation(intent_id, &scope_id)?,
                harness: else_harness::select_harness(&spec.outcome, harness_hint),
            });
        }

        if let Some((scope_id, run_id)) = active_run {
            return Ok(KernelAction::WaitForRun { scope_id, run_id });
        }
        if let Some(due_at) = projection.next_timer_at() {
            return Ok(KernelAction::WaitForTimer { due_at });
        }
        if !projection.open_requests.is_empty() {
            return Ok(KernelAction::WaitForAttentionWindow {
                requests: projection.open_requests.keys().cloned().collect(),
            });
        }
        Ok(KernelAction::Idle)
    }

    /// 执行一次 Kernel 裁决。Waiting 状态不产生任何事件。
    pub fn advance(
        &self,
        intent_id: &IntentId,
        harness_hint: Option<&str>,
    ) -> Result<IntentProjection> {
        match self.kernel_step(intent_id, harness_hint)? {
            KernelAction::Idle
            | KernelAction::WaitForUser { .. }
            | KernelAction::WaitForAttentionWindow { .. }
            | KernelAction::WaitForRun { .. }
            | KernelAction::WaitForTimer { .. } => self.projection(intent_id),
            KernelAction::FireTimer { timer_id } => {
                self.fire_timer(intent_id, &timer_id)?;
                self.projection(intent_id)
            }
            KernelAction::Run {
                scope_id,
                generation,
                harness,
                ..
            } => self.execute_kernel_run(intent_id, &scope_id, generation, harness),
        }
    }

    /// 连续推进，直到等待用户、等待 Run/Timer、空闲或达到步数上限。
    pub fn drive(
        &self,
        intent_id: &IntentId,
        harness_hint: Option<&str>,
        max_steps: u64,
    ) -> Result<KernelDrive> {
        let mut steps = 0;
        loop {
            if steps >= max_steps {
                return Ok(KernelDrive {
                    projection: self.projection(intent_id)?,
                    stopped: KernelDriveStop::StepLimit,
                    steps,
                });
            }
            let action = self.kernel_step(intent_id, harness_hint)?;
            let stopped = match &action {
                KernelAction::Idle => Some(KernelDriveStop::Idle),
                KernelAction::WaitForUser { .. } => Some(KernelDriveStop::WaitForUser),
                KernelAction::WaitForAttentionWindow { .. } => {
                    Some(KernelDriveStop::WaitForAttentionWindow)
                }
                KernelAction::WaitForRun { .. } => Some(KernelDriveStop::WaitForRun),
                KernelAction::WaitForTimer { .. } => Some(KernelDriveStop::WaitForTimer),
                KernelAction::FireTimer { .. } | KernelAction::Run { .. } => None,
            };
            if let Some(stopped) = stopped {
                return Ok(KernelDrive {
                    projection: self.projection(intent_id)?,
                    stopped,
                    steps,
                });
            }
            match action {
                KernelAction::FireTimer { timer_id } => {
                    self.fire_timer(intent_id, &timer_id)?;
                }
                KernelAction::Run {
                    scope_id,
                    generation,
                    harness,
                    ..
                } => {
                    self.execute_kernel_run(intent_id, &scope_id, generation, harness)?;
                }
                _ => unreachable!("waiting actions returned above"),
            }
            steps += 1;
        }
    }

    fn execute_kernel_run(
        &self,
        intent_id: &IntentId,
        scope_id: &ScopeId,
        generation: u64,
        harness: &str,
    ) -> Result<IntentProjection> {
        if harness == else_harness::SCRIPTED {
            return if self.projection(intent_id)?.root_scope_id.as_ref() == Some(scope_id) {
                self.run(intent_id)
            } else {
                self.run_scope_at_generation(intent_id, scope_id, generation)
            };
        }
        let adapter = else_harness::find_adapter(harness)
            .ok_or_else(|| ApiError::NotFound(format!("harness {harness}")))?;
        self.dispatch_scope_adapter(intent_id, scope_id, adapter.as_ref())
    }

    fn scope_scheduling_order(&self, projection: &IntentProjection) -> Vec<ScopeId> {
        let mut order = projection.scope_order.clone();
        for scope_id in projection.scopes.keys() {
            if !order.contains(scope_id) {
                order.push(scope_id.clone());
            }
        }
        order
    }
}
