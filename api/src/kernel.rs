//! Thread Kernel：从持久事件推导下一步，并连续推进到等待态。

use chrono::{DateTime, Utc};
use else_ledger::ThreadProjection;
use else_protocol::attention::AttentionWindowKind;
use else_protocol::event::ScopeState;
use else_protocol::ids::{AttentionId, AttentionWindowId, RunId, ScopeId, ThreadId, TimerId};
use else_protocol::thread::ThreadState;

use crate::attention::{next_attention_action, AttentionScheduleAction};
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
    WaitForExternal {
        reason: Option<String>,
    },
    BlockedByDependency {
        scope_id: ScopeId,
        dependencies: Vec<ScopeId>,
    },
    ConvergeDependency {
        scope_id: ScopeId,
        dependencies: Vec<ScopeId>,
    },
    FireTimer {
        timer_id: TimerId,
    },
    CloseAttentionWindow {
        window_id: AttentionWindowId,
    },
    ScheduleAttentionWindow {
        window_id: AttentionWindowId,
        timer_id: TimerId,
        due_at: DateTime<Utc>,
    },
    OpenAttentionWindow {
        kind: AttentionWindowKind,
        request_ids: Vec<AttentionId>,
        due_at: DateTime<Utc>,
    },
    AddAttentionWindowMembers {
        window_id: AttentionWindowId,
        request_ids: Vec<AttentionId>,
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
    WaitForExternal,
    BlockedByDependency,
    StepLimit,
}

#[derive(Debug, Clone, PartialEq)]
pub struct KernelDrive {
    pub projection: ThreadProjection,
    pub stopped: KernelDriveStop,
    pub steps: u64,
}

impl Runtime {
    /// 从 Event Ledger 推导当前 Thread 的下一步。
    /// 该函数不执行副作用，可用于预演、调试与调度循环。
    pub fn kernel_step(
        &self,
        thread_id: &ThreadId,
        harness_hint: Option<&str>,
    ) -> Result<KernelAction> {
        let projection = self.projection(thread_id)?;
        if matches!(
            projection.state,
            Some(ThreadState::Completed | ThreadState::Failed | ThreadState::Cancelled)
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
        if let Some(action) = next_attention_action(&projection, now) {
            return Ok(match action {
                AttentionScheduleAction::Close { window_id } => {
                    KernelAction::CloseAttentionWindow { window_id }
                }
                AttentionScheduleAction::Schedule { window_id, due_at } => {
                    let timer_id = projection
                        .attention_windows
                        .get(&window_id)
                        .map(|window| window.timer_id.clone())
                        .ok_or_else(|| {
                            ApiError::NotFound(format!("attention window {window_id}"))
                        })?;
                    KernelAction::ScheduleAttentionWindow {
                        window_id,
                        timer_id,
                        due_at,
                    }
                }
                AttentionScheduleAction::Open {
                    kind,
                    request_ids,
                    due_at,
                } => KernelAction::OpenAttentionWindow {
                    kind,
                    request_ids,
                    due_at,
                },
                AttentionScheduleAction::AddMembers {
                    window_id,
                    request_ids,
                } => KernelAction::AddAttentionWindowMembers {
                    window_id,
                    request_ids,
                },
            });
        }
        if projection.state == Some(ThreadState::WaitingExternal) {
            return if let Some(due_at) = projection.next_timer_at() {
                Ok(KernelAction::WaitForTimer { due_at })
            } else {
                Ok(KernelAction::WaitForExternal {
                    reason: projection.external_wait_reason.clone(),
                })
            };
        }

        let mut active_run = None;
        let mut ready = Vec::new();
        let mut blocked_by_dependency = Vec::new();
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
                ScopeState::Completed
                    | ScopeState::Failed
                    | ScopeState::Superseded
                    | ScopeState::Cancelled
            ) {
                continue;
            }
            let unresolved: Vec<_> = scope
                .depends_on
                .iter()
                .filter(|dependency| {
                    projection
                        .scopes
                        .get(*dependency)
                        .is_none_or(|dependency_scope| {
                            dependency_scope.state != ScopeState::Completed
                        })
                })
                .cloned()
                .collect();
            if !unresolved.is_empty() {
                let terminally_blocked = unresolved.iter().any(|dependency| {
                    projection
                        .scopes
                        .get(dependency)
                        .is_some_and(|dependency_scope| {
                            matches!(
                                dependency_scope.state,
                                ScopeState::Failed | ScopeState::Cancelled
                            )
                        })
                });
                if terminally_blocked {
                    blocked_by_dependency.push((scope_id.clone(), unresolved));
                }
                continue;
            }
            if let Some(run_id) = self.active_run_id(thread_id, &scope_id)? {
                active_run.get_or_insert((scope_id.clone(), run_id));
                continue;
            }
            ready.push((scope.last_run_started_seq.unwrap_or(0), index, scope_id));
        }
        ready.sort_by_key(|(last_started, order, _)| (*last_started, *order));
        if let Some((scope_id, run_id)) = active_run {
            return Ok(KernelAction::WaitForRun { scope_id, run_id });
        }
        if let Some((scope_id, dependencies)) = blocked_by_dependency.into_iter().next() {
            return Ok(KernelAction::ConvergeDependency {
                scope_id,
                dependencies,
            });
        }
        if let Some((_, _, scope_id)) = ready.into_iter().next() {
            let spec = scope_spec(&projection, &scope_id)
                .ok_or_else(|| ApiError::NotFound(format!("scope {scope_id}")))?;
            let selected = else_harness::select_harness(&spec.outcome, harness_hint);
            let harness = if selected == else_harness::SCRIPTED {
                selected
            } else {
                else_harness::find_adapter(selected)
                    .filter(|adapter| self.harness_allowed(&spec, adapter.as_ref()))
                    .map_or(else_harness::SCRIPTED, |_| selected)
            };
            return Ok(KernelAction::Run {
                scope_id: scope_id.clone(),
                generation: self.next_scope_generation(thread_id, &scope_id)?,
                harness,
            });
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
        thread_id: &ThreadId,
        harness_hint: Option<&str>,
    ) -> Result<ThreadProjection> {
        match self.kernel_step(thread_id, harness_hint)? {
            KernelAction::Idle
            | KernelAction::WaitForUser { .. }
            | KernelAction::WaitForAttentionWindow { .. }
            | KernelAction::WaitForRun { .. }
            | KernelAction::WaitForTimer { .. }
            | KernelAction::WaitForExternal { .. }
            | KernelAction::BlockedByDependency { .. } => self.projection(thread_id),
            KernelAction::ConvergeDependency {
                scope_id,
                dependencies,
            } => self.converge_dependency_failure(thread_id, &scope_id, &dependencies),
            KernelAction::FireTimer { timer_id } => {
                self.fire_timer(thread_id, &timer_id)?;
                self.projection(thread_id)
            }
            KernelAction::CloseAttentionWindow { window_id } => {
                self.close_attention_window(thread_id, &window_id)?;
                self.projection(thread_id)
            }
            KernelAction::ScheduleAttentionWindow {
                window_id,
                timer_id,
                due_at,
            } => {
                self.schedule_attention_window_timer(thread_id, &window_id, &timer_id, due_at)?;
                self.projection(thread_id)
            }
            KernelAction::OpenAttentionWindow {
                kind,
                request_ids,
                due_at,
            } => {
                self.open_attention_window(thread_id, kind, request_ids, due_at)?;
                self.projection(thread_id)
            }
            KernelAction::AddAttentionWindowMembers {
                window_id,
                request_ids,
            } => {
                self.add_attention_window_members(thread_id, &window_id, request_ids)?;
                self.projection(thread_id)
            }
            KernelAction::Run {
                scope_id,
                generation,
                harness,
                ..
            } => self.execute_kernel_run(thread_id, &scope_id, generation, harness),
        }
    }

    /// 连续推进，直到等待用户、等待 Run/Timer、空闲或达到步数上限。
    pub fn drive(
        &self,
        thread_id: &ThreadId,
        harness_hint: Option<&str>,
        max_steps: u64,
    ) -> Result<KernelDrive> {
        self.drive_inner(thread_id, harness_hint, max_steps, true)
    }

    /// 只推进调度层：定时器、注意力窗口、依赖收敛，**绝不启动 Run**。
    ///
    /// 后台 reconciler 用它。否则一个刚创建、用户还没来得及配置的 Thread
    /// 会被后台先跑一轮，用户传的 harness 与授权永远晚一步。
    pub fn drive_scheduling(&self, thread_id: &ThreadId, max_steps: u64) -> Result<KernelDrive> {
        self.drive_inner(thread_id, None, max_steps, false)
    }

    fn drive_inner(
        &self,
        thread_id: &ThreadId,
        harness_hint: Option<&str>,
        max_steps: u64,
        allow_runs: bool,
    ) -> Result<KernelDrive> {
        let mut steps = 0;
        loop {
            if steps >= max_steps {
                return Ok(KernelDrive {
                    projection: self.projection(thread_id)?,
                    stopped: KernelDriveStop::StepLimit,
                    steps,
                });
            }
            let action = self.kernel_step(thread_id, harness_hint)?;
            let stopped = match &action {
                KernelAction::Idle => Some(KernelDriveStop::Idle),
                KernelAction::WaitForUser { .. } => Some(KernelDriveStop::WaitForUser),
                KernelAction::WaitForAttentionWindow { .. } => {
                    Some(KernelDriveStop::WaitForAttentionWindow)
                }
                KernelAction::WaitForRun { .. } => Some(KernelDriveStop::WaitForRun),
                KernelAction::WaitForTimer { .. } => Some(KernelDriveStop::WaitForTimer),
                KernelAction::WaitForExternal { .. } => Some(KernelDriveStop::WaitForExternal),
                KernelAction::BlockedByDependency { .. } => {
                    Some(KernelDriveStop::BlockedByDependency)
                }
                KernelAction::ConvergeDependency { .. } => None,
                KernelAction::Run { .. } if !allow_runs => Some(KernelDriveStop::WaitForRun),
                KernelAction::FireTimer { .. }
                | KernelAction::CloseAttentionWindow { .. }
                | KernelAction::ScheduleAttentionWindow { .. }
                | KernelAction::OpenAttentionWindow { .. }
                | KernelAction::AddAttentionWindowMembers { .. }
                | KernelAction::Run { .. } => None,
            };
            if let Some(stopped) = stopped {
                return Ok(KernelDrive {
                    projection: self.projection(thread_id)?,
                    stopped,
                    steps,
                });
            }
            match action {
                KernelAction::FireTimer { timer_id } => {
                    self.fire_timer(thread_id, &timer_id)?;
                }
                KernelAction::CloseAttentionWindow { window_id } => {
                    self.close_attention_window(thread_id, &window_id)?;
                }
                KernelAction::ScheduleAttentionWindow {
                    window_id,
                    timer_id,
                    due_at,
                } => {
                    self.schedule_attention_window_timer(thread_id, &window_id, &timer_id, due_at)?;
                }
                KernelAction::OpenAttentionWindow {
                    kind,
                    request_ids,
                    due_at,
                } => {
                    self.open_attention_window(thread_id, kind, request_ids, due_at)?;
                }
                KernelAction::AddAttentionWindowMembers {
                    window_id,
                    request_ids,
                } => {
                    self.add_attention_window_members(thread_id, &window_id, request_ids)?;
                }
                KernelAction::ConvergeDependency {
                    scope_id,
                    dependencies,
                } => {
                    self.converge_dependency_failure(thread_id, &scope_id, &dependencies)?;
                }
                KernelAction::Run {
                    scope_id,
                    generation,
                    harness,
                    ..
                } => {
                    self.execute_kernel_run(thread_id, &scope_id, generation, harness)?;
                }
                _ => unreachable!("waiting actions returned above"),
            }
            steps += 1;
        }
    }

    fn execute_kernel_run(
        &self,
        thread_id: &ThreadId,
        scope_id: &ScopeId,
        generation: u64,
        harness: &str,
    ) -> Result<ThreadProjection> {
        if harness == else_harness::SCRIPTED {
            return if self.projection(thread_id)?.root_scope_id.as_ref() == Some(scope_id) {
                self.run(thread_id)
            } else {
                self.run_scope_at_generation(thread_id, scope_id, generation)
            };
        }
        let adapter = else_harness::find_adapter(harness)
            .ok_or_else(|| ApiError::NotFound(format!("harness {harness}")))?;
        self.dispatch_scope_adapter(thread_id, scope_id, adapter.as_ref())
    }

    fn scope_scheduling_order(&self, projection: &ThreadProjection) -> Vec<ScopeId> {
        let mut order = projection.scope_order.clone();
        for scope_id in projection.scopes.keys() {
            if !order.contains(scope_id) {
                order.push(scope_id.clone());
            }
        }
        order
    }
}
