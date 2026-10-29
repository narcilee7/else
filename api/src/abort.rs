//! M1 Abort：持久化状态机、在途副作用收敛与崩溃恢复。
//!
//! Cancelled 只表示执行安全收敛，不表示外部世界已经回滚。

use std::collections::{BTreeMap, BTreeSet};

use else_gateway::ExecContext;
use else_ledger::IntentProjection;
use else_protocol::attention::{Decision, DecisionChoice};
use else_protocol::effect::{EffectConclusion, ToolCall};
use else_protocol::event::{Actor, EventPayload, ScopeState};
use else_protocol::ids::{IntentId, RunId, ToolCallId};
use else_protocol::intent::IntentState;

use crate::{ApiError, Result, Runtime};

impl Runtime {
    /// 请求取消当前执行。重复 Abort 与终态 Abort 都是幂等操作。
    pub fn abort(&self, intent_id: &IntentId, reason: &str) -> Result<IntentProjection> {
        let projection = self.projection(intent_id)?;
        let spec = projection
            .spec
            .clone()
            .ok_or_else(|| ApiError::IntentNotFound(intent_id.clone()))?;
        if matches!(
            projection.state,
            Some(IntentState::Completed | IntentState::Failed | IntentState::Cancelled)
        ) {
            return Ok(projection);
        }

        let events = self.ledger.read_intent(intent_id)?;
        let source = events
            .iter()
            .rev()
            .find(|event| event.envelope.run_id.is_some())
            .or_else(|| events.last())
            .ok_or_else(|| ApiError::IntentNotFound(intent_id.clone()))?;
        let ctx = ExecContext {
            spec,
            scope_id: source.envelope.scope_id.clone(),
            run_id: source.envelope.run_id.clone().unwrap_or_else(RunId::new),
            generation: source.envelope.generation,
        };
        ctx.record(
            &self.ledger,
            Actor::User,
            EventPayload::ScopeAbortRequested {
                reason: reason.to_string(),
            },
        )?;
        self.recover_abort(intent_id)?;
        self.projection(intent_id)
    }

    /// 打开 Runtime 时恢复所有未完成的 Abort。状态完全来自事件，不依赖内存。
    pub(crate) fn recover_aborts(&self) -> Result<()> {
        let intent_ids: BTreeSet<_> = self
            .ledger
            .read_all()?
            .into_iter()
            .map(|event| event.envelope.intent_id)
            .collect();
        for intent_id in intent_ids {
            self.recover_abort(&intent_id)?;
        }
        Ok(())
    }

    /// 从最后一次 ScopeAbortRequested 继续；每一步按事件是否存在决定，天然可重入。
    fn recover_abort(&self, intent_id: &IntentId) -> Result<()> {
        let events = self.ledger.read_intent(intent_id)?;
        let Some(abort) = events.iter().rev().find(|event| {
            matches!(
                event.envelope.payload,
                EventPayload::ScopeAbortRequested { .. }
            )
        }) else {
            return Ok(());
        };
        let abort_seq = abort.seq;
        let scope_id = abort.envelope.scope_id.clone();
        let generation = abort.envelope.generation;
        let after_abort = |event: &&else_ledger::StoredEvent| {
            event.seq > abort_seq
                && event.envelope.scope_id == scope_id
                && event.envelope.generation == generation
        };
        let scope_cancelled = events.iter().filter(after_abort).any(|event| {
            matches!(
                event.envelope.payload,
                EventPayload::ScopeStateChanged {
                    state: ScopeState::Cancelled
                }
            )
        });
        let intent_cancelled = events.iter().filter(after_abort).any(|event| {
            matches!(
                event.envelope.payload,
                EventPayload::IntentStateChanged {
                    state: IntentState::Cancelled
                }
            )
        });
        if scope_cancelled && intent_cancelled {
            return Ok(());
        }

        let projection = self.projection(intent_id)?;
        let spec = projection
            .spec
            .ok_or_else(|| ApiError::IntentNotFound(intent_id.clone()))?;
        let ctx = ExecContext {
            spec,
            scope_id: scope_id.clone(),
            run_id: abort.envelope.run_id.clone().unwrap_or_else(RunId::new),
            generation,
        };
        let has_scope_state = |state: ScopeState| {
            events.iter().filter(after_abort).any(|event| {
                matches!(
                    event.envelope.payload,
                    EventPayload::ScopeStateChanged { state: actual } if actual == state
                )
            })
        };
        for state in [ScopeState::Cancelling, ScopeState::Fenced] {
            if !has_scope_state(state) {
                ctx.record(
                    &self.ledger,
                    Actor::System,
                    EventPayload::ScopeStateChanged { state },
                )?;
            }
        }

        // 活跃 Run 必须有关闭事件，不能在重放后看起来仍在执行。
        let mut started = BTreeSet::new();
        let mut finished = BTreeSet::new();
        for event in events.iter().filter(|event| event.seq < abort_seq) {
            match &event.envelope.payload {
                EventPayload::RunStarted { run_id, .. } => {
                    started.insert(run_id.clone());
                }
                EventPayload::RunFinished { run_id, .. } => {
                    finished.insert(run_id.clone());
                }
                _ => {}
            }
        }
        for run_id in started.difference(&finished) {
            ctx.record(
                &self.ledger,
                Actor::System,
                EventPayload::RunFinished {
                    run_id: run_id.clone(),
                    success: false,
                },
            )?;
        }

        // 只收尾 Abort 之前已在途的调用；Unknown 是结论，绝不假装已回滚。
        let inflight = inflight_before(&events, abort_seq);
        let concluded: BTreeSet<_> = events
            .iter()
            .filter(after_abort)
            .filter_map(|event| match &event.envelope.payload {
                EventPayload::SideEffectConcluded { call_id, .. } => Some(call_id.clone()),
                _ => None,
            })
            .collect();
        for call in inflight {
            if !concluded.contains(&call.id) {
                ctx.record(
                    &self.ledger,
                    Actor::System,
                    EventPayload::SideEffectConcluded {
                        call_id: call.id,
                        conclusion: EffectConclusion::Unknown,
                        note: format!(
                            "Abort 时 {} 的结果未知；未盲目重试，也未声称已补偿",
                            call.tool
                        ),
                    },
                )?;
            }
        }

        if !has_scope_state(ScopeState::Compensating) {
            ctx.record(
                &self.ledger,
                Actor::System,
                EventPayload::ScopeStateChanged {
                    state: ScopeState::Compensating,
                },
            )?;
        }

        // 取消同时关闭旧 Decision Queue，避免用户批准一个已经失效的执行。
        for request_id in projection.open_requests.keys() {
            ctx.record(
                &self.ledger,
                Actor::System,
                EventPayload::DecisionResolved {
                    decision: Decision {
                        request_id: request_id.clone(),
                        choice: DecisionChoice::Dismissed,
                        rationale: Some("Intent 已取消，旧决策请求失效".into()),
                        decided_by: Actor::System,
                    },
                },
            )?;
        }

        if !scope_cancelled {
            ctx.record(
                &self.ledger,
                Actor::System,
                EventPayload::ScopeStateChanged {
                    state: ScopeState::Cancelled,
                },
            )?;
        }
        if !intent_cancelled {
            ctx.record(
                &self.ledger,
                Actor::System,
                EventPayload::IntentStateChanged {
                    state: IntentState::Cancelled,
                },
            )?;
        }
        Ok(())
    }

    /// 测试故障注入：持久化 N 条事件后模拟进程被 kill。
    #[doc(hidden)]
    pub fn crash_after_appends(&self, appends: u64) {
        self.ledger.crash_after_appends(appends);
    }
}

fn inflight_before(events: &[else_ledger::StoredEvent], abort_seq: u64) -> Vec<ToolCall> {
    let mut called: BTreeMap<ToolCallId, ToolCall> = BTreeMap::new();
    let mut done = BTreeSet::new();
    for event in events.iter().filter(|event| event.seq < abort_seq) {
        match &event.envelope.payload {
            EventPayload::ToolCalled { call } => {
                called.insert(call.id.clone(), call.clone());
            }
            EventPayload::ToolCompleted { call_id, .. } => {
                done.insert(call_id.clone());
            }
            _ => {}
        }
    }
    called
        .into_values()
        .filter(|call| !done.contains(&call.id))
        .collect()
}
