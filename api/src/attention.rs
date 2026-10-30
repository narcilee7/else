//! Attention Window 调度：Batch / Defer 的持久窗口、Timer 与幂等关闭。

use chrono::{DateTime, Duration, Utc};
use else_ledger::IntentProjection;
use else_protocol::attention::{AttentionRequest, AttentionWindow, AttentionWindowKind, Ruling};
use else_protocol::event::{Actor, EventEnvelope, EventPayload};
use else_protocol::ids::{AttentionId, AttentionWindowId, IntentId, ScopeId, TimerId};

use crate::{Result, Runtime};

/// Kernel 为补齐 Attention Window 需要执行的一次确定性动作。
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum AttentionScheduleAction {
    Close {
        window_id: AttentionWindowId,
    },
    Schedule {
        window_id: AttentionWindowId,
        due_at: DateTime<Utc>,
    },
    Open {
        kind: AttentionWindowKind,
        request_ids: Vec<AttentionId>,
        due_at: DateTime<Utc>,
    },
    AddMembers {
        window_id: AttentionWindowId,
        request_ids: Vec<AttentionId>,
    },
}

/// 从投影推导下一个窗口动作。顺序保证截止窗口先关闭，之后才允许新请求
/// 进入新窗口，避免晚到请求被塞进已经应关闭的 Batch。
pub(crate) fn next_attention_action(
    projection: &IntentProjection,
    now: DateTime<Utc>,
) -> Option<AttentionScheduleAction> {
    let mut windows: Vec<_> = projection.attention_windows.values().collect();
    windows.sort_by_key(|window| (window.due_at, window.id.clone()));

    for window in &windows {
        let has_open_request = window
            .request_ids
            .iter()
            .any(|request_id| projection.open_requests.contains_key(request_id));
        let is_due = window.closed_at.is_none() && window.due_at <= now;
        if window.closed_at.is_none() && (!has_open_request || is_due) {
            return Some(AttentionScheduleAction::Close {
                window_id: window.id.clone(),
            });
        }
    }

    for window in &windows {
        if window.closed_at.is_none()
            && window.due_at > now
            && !projection.timers.contains_key(&window.timer_id)
        {
            return Some(AttentionScheduleAction::Schedule {
                window_id: window.id.clone(),
                due_at: window.due_at,
            });
        }
    }

    let batch_requests: Vec<_> = projection
        .open_requests
        .values()
        .filter(|request| {
            matches!(
                projection.attention_rulings.get(&request.id),
                Some(Ruling::Batch)
            ) && projection
                .attention_window_for_request(&request.id)
                .is_none()
        })
        .map(|request| request.id.clone())
        .collect();
    if !batch_requests.is_empty() {
        if let Some(window) = windows.iter().rev().find(|window| {
            window.kind == AttentionWindowKind::Batch
                && window.closed_at.is_none()
                && window.due_at > now
        }) {
            return Some(AttentionScheduleAction::AddMembers {
                window_id: window.id.clone(),
                request_ids: batch_requests,
            });
        }
        let interval_minutes = projection
            .spec
            .as_ref()
            .map(|spec| i64::from(spec.attention_policy.batch_interval_minutes.max(1)))
            .unwrap_or(30);
        return Some(AttentionScheduleAction::Open {
            kind: AttentionWindowKind::Batch,
            request_ids: batch_requests,
            due_at: now + Duration::minutes(interval_minutes),
        });
    }

    let mut deferred: Vec<_> = projection
        .open_requests
        .values()
        .filter_map(
            |request| match projection.attention_rulings.get(&request.id) {
                Some(Ruling::Defer { until })
                    if projection
                        .attention_window_for_request(&request.id)
                        .is_none() =>
                {
                    Some((*until, request.id.clone()))
                }
                _ => None,
            },
        )
        .collect();
    deferred.sort();
    if let Some((due_at, request_id)) = deferred.into_iter().next() {
        return Some(AttentionScheduleAction::Open {
            kind: AttentionWindowKind::Defer,
            request_ids: vec![request_id],
            due_at,
        });
    }

    None
}

impl Runtime {
    /// 连接器或 Subagent 提交注意力请求的统一入口。裁决与会话状态先落账，
    /// Batch / Defer 的实际通知由 Kernel 窗口驱动，而不是调用方直接展示。
    pub fn submit_attention_request(
        &self,
        intent_id: &IntentId,
        request: &AttentionRequest,
    ) -> Result<AttentionId> {
        let projection = self.projection(intent_id)?;
        let scope_id = projection
            .root_scope_id
            .ok_or_else(|| crate::ApiError::IntentNotFound(intent_id.clone()))?;
        self.submit_attention_request_for_scope(intent_id, &scope_id, request)
    }

    /// 为指定 Scope 提交请求；Intent Revision 取消该分支时请求会同步失效。
    pub fn submit_attention_request_for_scope(
        &self,
        intent_id: &IntentId,
        scope_id: &ScopeId,
        request: &AttentionRequest,
    ) -> Result<AttentionId> {
        let projection = self.projection(intent_id)?;
        let policy = projection
            .spec
            .as_ref()
            .map(|spec| &spec.attention_policy)
            .ok_or_else(|| crate::ApiError::IntentNotFound(intent_id.clone()))?;
        let ruling = else_attention::AttentionKernel.rule(request, policy, Utc::now());
        let scope = projection
            .scopes
            .get(scope_id)
            .ok_or_else(|| crate::ApiError::NotFound(format!("scope {scope_id}")))?;
        let generation = scope.generation;
        let revision = projection.spec.as_ref().map_or(1, |spec| spec.revision);
        self.ledger.append(&EventEnvelope::new(
            intent_id.clone(),
            scope_id.clone(),
            generation,
            revision,
            Actor::System,
            EventPayload::AttentionRequested {
                request: request.clone(),
            },
        ))?;
        self.ledger.append(&EventEnvelope::new(
            intent_id.clone(),
            scope_id.clone(),
            generation,
            revision,
            Actor::System,
            EventPayload::AttentionRuled {
                request_id: request.id.clone(),
                ruling,
            },
        ))?;
        self.ledger.append(&EventEnvelope::new(
            intent_id.clone(),
            scope_id.clone(),
            generation,
            revision,
            Actor::System,
            EventPayload::IntentStateChanged {
                state: else_protocol::intent::IntentState::NeedsDecision,
            },
        ))?;
        Ok(request.id.clone())
    }

    pub(crate) fn open_attention_window(
        &self,
        intent_id: &IntentId,
        kind: AttentionWindowKind,
        request_ids: Vec<AttentionId>,
        due_at: DateTime<Utc>,
    ) -> Result<()> {
        let (scope_id, generation, revision) = self.kernel_context(intent_id)?;
        let opened_at = Utc::now();
        let id = AttentionWindowId::new();
        let timer_id = TimerId::from(format!("attention-window/{id}"));
        let window = AttentionWindow {
            id: id.clone(),
            kind,
            opened_at,
            due_at,
            timer_id: timer_id.clone(),
            closed_at: None,
            delivered_at: None,
            request_ids,
        };
        self.ledger.append(&EventEnvelope::new(
            intent_id.clone(),
            scope_id.clone(),
            generation,
            revision,
            Actor::System,
            EventPayload::AttentionWindowOpened { window },
        ))?;
        self.schedule_attention_window_timer(intent_id, &id, &timer_id, due_at)
    }

    pub(crate) fn add_attention_window_members(
        &self,
        intent_id: &IntentId,
        window_id: &AttentionWindowId,
        request_ids: Vec<AttentionId>,
    ) -> Result<()> {
        let projection = self.projection(intent_id)?;
        let Some(window) = projection.attention_windows.get(window_id) else {
            return Ok(());
        };
        if window.closed_at.is_some() {
            return Ok(());
        }
        let (scope_id, generation, revision) = self.kernel_context(intent_id)?;
        for request_id in request_ids {
            if window.request_ids.contains(&request_id) {
                continue;
            }
            self.ledger.append(&EventEnvelope::new(
                intent_id.clone(),
                scope_id.clone(),
                generation,
                revision,
                Actor::System,
                EventPayload::AttentionWindowMemberAdded {
                    window_id: window_id.clone(),
                    request_id,
                },
            ))?;
        }
        Ok(())
    }

    pub(crate) fn close_attention_window(
        &self,
        intent_id: &IntentId,
        window_id: &AttentionWindowId,
    ) -> Result<()> {
        let projection = self.projection(intent_id)?;
        let Some(window) = projection.attention_windows.get(window_id) else {
            return Ok(());
        };
        if window.closed_at.is_some() {
            return Ok(());
        }
        let (scope_id, generation, revision) = self.kernel_context(intent_id)?;
        let closed_at = Utc::now();
        let delivered_at = (window.due_at <= closed_at
            && window
                .request_ids
                .iter()
                .any(|request_id| projection.open_requests.contains_key(request_id)))
        .then_some(closed_at);
        if projection.timers.contains_key(&window.timer_id) {
            self.ledger.append(&EventEnvelope::new(
                intent_id.clone(),
                scope_id.clone(),
                generation,
                revision,
                Actor::System,
                EventPayload::TimerCancelled {
                    timer_id: window.timer_id.clone(),
                    reason: format!("attention window {window_id} closed"),
                },
            ))?;
        }
        self.ledger.append(&EventEnvelope::new(
            intent_id.clone(),
            scope_id,
            generation,
            revision,
            Actor::System,
            EventPayload::AttentionWindowClosed {
                window_id: window_id.clone(),
                closed_at,
                delivered_at,
            },
        ))?;
        Ok(())
    }

    pub(crate) fn schedule_attention_window_timer(
        &self,
        intent_id: &IntentId,
        window_id: &AttentionWindowId,
        timer_id: &TimerId,
        due_at: DateTime<Utc>,
    ) -> Result<()> {
        let projection = self.projection(intent_id)?;
        if projection.timers.contains_key(timer_id) {
            return Ok(());
        }
        let (scope_id, generation, revision) = self.kernel_context(intent_id)?;
        self.ledger.append(&EventEnvelope::new(
            intent_id.clone(),
            scope_id,
            generation,
            revision,
            Actor::System,
            EventPayload::TimerScheduled {
                timer_id: timer_id.clone(),
                fire_at: due_at,
                reason: format!("attention window {window_id}"),
            },
        ))?;
        Ok(())
    }

    /// Decision / Abort / Revision 改变成员后，关闭已经没有任何未决请求的窗口，
    /// 同时取消尚未触发的 Timer，避免后续空唤醒。
    pub(crate) fn close_empty_attention_windows(&self, intent_id: &IntentId) -> Result<()> {
        loop {
            let projection = self.projection(intent_id)?;
            let stale = projection
                .attention_windows
                .values()
                .find(|window| {
                    window.closed_at.is_none()
                        && !window
                            .request_ids
                            .iter()
                            .any(|request_id| projection.open_requests.contains_key(request_id))
                })
                .map(|window| window.id.clone());
            let Some(window_id) = stale else {
                return Ok(());
            };
            self.close_attention_window(intent_id, &window_id)?;
        }
    }

    /// Scope 被取消或 Revision 使分支失效时，关闭其未决注意力请求。
    /// 请求和窗口都保留为历史，不做原地删除。
    pub(crate) fn expire_requests_for_cancelled_scopes(&self, intent_id: &IntentId) -> Result<()> {
        let projection = self.projection(intent_id)?;
        let events = self.events(intent_id)?;
        let mut expired = 0_u64;
        for request_id in projection.open_requests.keys() {
            let Some(request_event) = events.iter().rev().find(|event| {
                matches!(
                    &event.envelope.payload,
                    EventPayload::AttentionRequested { request }
                        if &request.id == request_id
                )
            }) else {
                continue;
            };
            if !matches!(
                projection
                    .scopes
                    .get(&request_event.envelope.scope_id)
                    .map(|scope| scope.state),
                Some(else_protocol::event::ScopeState::Cancelled)
                    | Some(else_protocol::event::ScopeState::Failed)
            ) {
                continue;
            }
            self.ledger.append(&EventEnvelope::new(
                intent_id.clone(),
                request_event.envelope.scope_id.clone(),
                request_event.envelope.generation,
                request_event.envelope.intent_revision,
                Actor::System,
                EventPayload::DecisionResolved {
                    decision: else_protocol::attention::Decision {
                        request_id: request_id.clone(),
                        choice: else_protocol::attention::DecisionChoice::Dismissed,
                        rationale: Some("所属 Scope 已取消或失效".into()),
                        decided_by: Actor::System,
                    },
                },
            ))?;
            expired += 1;
        }
        if expired > 0 {
            self.close_empty_attention_windows(intent_id)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{IntentRevision, KernelAction, KernelDriveStop};
    use else_protocol::attention::{AttentionKind, DecisionOption, ExpectedLoss};
    use else_protocol::effect::SideEffectKind;
    use else_protocol::event::ScopeState;

    fn attention_request(loss: ExpectedLoss, blocking: bool, reversible: bool) -> AttentionRequest {
        AttentionRequest {
            id: AttentionId::new(),
            kind: AttentionKind::Judgment,
            summary: "选择下一步".into(),
            blocked_reason: "需要本人判断".into(),
            expected_loss: loss,
            blocking,
            reversible,
            decide_by: None,
            options: vec![DecisionOption {
                id: "approve".into(),
                label: "继续".into(),
                recommended: true,
            }],
            default_option: Some("approve".into()),
            related_call_key: None,
        }
    }

    #[test]
    fn batch_window_opens_once_and_aggregates_new_requests() {
        let dir = tempfile::tempdir().unwrap();
        let runtime = Runtime::open(dir.path()).unwrap();
        let spec = runtime.create_intent("整理待办", None).unwrap();
        runtime.wait_for_external(&spec.id, "等待批量判断").unwrap();
        let first = attention_request(ExpectedLoss::Low, false, false);
        let second = attention_request(ExpectedLoss::Low, false, false);
        let first_id = first.id.clone();
        let second_id = second.id.clone();
        runtime.submit_attention_request(&spec.id, &first).unwrap();
        runtime.submit_attention_request(&spec.id, &second).unwrap();

        runtime.advance(&spec.id, None).unwrap();
        let projection = runtime.projection(&spec.id).unwrap();
        assert_eq!(projection.attention_windows.len(), 1);
        assert_eq!(projection.timers.len(), 1);
        let window = projection.attention_windows.values().next().unwrap();
        assert_eq!(window.kind, AttentionWindowKind::Batch);
        assert!(window.request_ids.contains(&first_id));
        assert!(window.request_ids.contains(&second_id));
        assert!(window.closed_at.is_none());
        assert!(projection.actionable_requests(Utc::now()).is_empty());

        // 重复推进/轮询不会创建新的窗口或 Timer。
        runtime.advance(&spec.id, None).unwrap();
        let projection = runtime.projection(&spec.id).unwrap();
        assert_eq!(projection.attention_windows.len(), 1);
        assert_eq!(projection.timers.len(), 1);
    }

    #[test]
    fn due_window_fires_timer_then_closes_exactly_once() {
        let dir = tempfile::tempdir().unwrap();
        let runtime = Runtime::open(dir.path()).unwrap();
        let spec = runtime.create_intent("整理待办", None).unwrap();
        runtime
            .wait_for_external(&spec.id, "等待注意力窗口")
            .unwrap();
        let request = attention_request(ExpectedLoss::Low, true, false);
        let request_id = request.id.clone();
        runtime
            .submit_attention_request(&spec.id, &request)
            .unwrap();
        runtime
            .open_attention_window(
                &spec.id,
                AttentionWindowKind::Batch,
                vec![request_id.clone()],
                Utc::now() - chrono::Duration::seconds(1),
            )
            .unwrap();

        let driven = runtime.drive(&spec.id, None, 8).unwrap();
        assert_eq!(driven.stopped, KernelDriveStop::WaitForUser);
        assert_eq!(driven.steps, 2, "fire timer, then close window");
        assert_eq!(driven.projection.actionable_requests(Utc::now()).len(), 1);
        assert_eq!(
            driven
                .projection
                .actionable_attention_windows(Utc::now())
                .len(),
            1
        );
        let window = driven.projection.attention_windows.values().next().unwrap();
        assert!(window.closed_at.is_some());
        assert!(driven.projection.timers.is_empty());
        let metrics = driven.projection.attention_metrics(Utc::now());
        assert_eq!(metrics.interventions, 1);
        assert_eq!(metrics.batch_notifications, 1);
        assert_eq!(metrics.batch_rate, Some(1.0));

        drop(runtime);
        let runtime = Runtime::open(dir.path()).unwrap();
        let replayed = runtime.projection(&spec.id).unwrap();
        assert_eq!(replayed.attention_windows.len(), 1);
        assert_eq!(replayed.actionable_requests(Utc::now()).len(), 1);
        assert!(matches!(
            runtime.kernel_step(&spec.id, None).unwrap(),
            KernelAction::WaitForUser { .. }
        ));
    }

    #[test]
    fn defer_has_a_persistent_timer_and_stays_hidden_until_closed() {
        let dir = tempfile::tempdir().unwrap();
        let runtime = Runtime::open(dir.path()).unwrap();
        let spec = runtime.create_intent("整理待办", None).unwrap();
        runtime.wait_for_external(&spec.id, "等待专注间隙").unwrap();
        let request = attention_request(ExpectedLoss::Medium, true, true);
        let request_id = request.id.clone();
        runtime
            .submit_attention_request(&spec.id, &request)
            .unwrap();

        runtime.advance(&spec.id, None).unwrap();
        let projection = runtime.projection(&spec.id).unwrap();
        let window = projection.attention_windows.values().next().unwrap();
        assert_eq!(window.kind, AttentionWindowKind::Defer);
        assert_eq!(window.request_ids, vec![request_id]);
        assert!(projection.timers.contains_key(&window.timer_id));
        assert!(projection.actionable_requests(Utc::now()).is_empty());
    }

    #[test]
    fn abort_closes_empty_attention_window_and_cancels_timer() {
        let dir = tempfile::tempdir().unwrap();
        let runtime = Runtime::open(dir.path()).unwrap();
        let spec = runtime.create_intent("整理待办", None).unwrap();
        runtime.wait_for_external(&spec.id, "等待判断").unwrap();
        let request = attention_request(ExpectedLoss::Low, false, false);
        runtime
            .submit_attention_request(&spec.id, &request)
            .unwrap();
        runtime.advance(&spec.id, None).unwrap();

        let projection = runtime.abort(&spec.id, "测试取消").unwrap();
        assert!(projection.open_requests.is_empty());
        assert!(projection.timers.is_empty());
        assert!(projection
            .attention_windows
            .values()
            .all(|window| window.closed_at.is_some()));
        assert_eq!(projection.attention_metrics(Utc::now()).interventions, 0);
    }

    #[test]
    fn revision_invalidates_requests_owned_by_cancelled_scopes() {
        let dir = tempfile::tempdir().unwrap();
        let runtime = Runtime::open(dir.path()).unwrap();
        let spec = runtime.create_intent("整理待办", None).unwrap();
        let root = runtime.projection(&spec.id).unwrap().root_scope_id.unwrap();
        let child = runtime
            .spawn_subagent(&spec.id, &root, "受影响分支", SideEffectKind::ReadOnly)
            .unwrap();
        let request = attention_request(ExpectedLoss::Low, false, false);
        runtime
            .submit_attention_request_for_scope(&spec.id, &child, &request)
            .unwrap();
        runtime.advance(&spec.id, None).unwrap();

        runtime
            .revise_intent(
                &spec.id,
                IntentRevision {
                    reason: "目标变化".into(),
                    ..IntentRevision::default()
                },
                &[],
            )
            .unwrap();
        let projection = runtime.projection(&spec.id).unwrap();
        assert_eq!(projection.scopes[&child].state, ScopeState::Cancelled);
        assert!(projection.open_requests.is_empty());
        assert!(projection.timers.is_empty());
        assert!(projection
            .attention_windows
            .values()
            .all(|window| window.closed_at.is_some()));
    }
}
