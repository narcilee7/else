//! M1 Abort：持久化状态机、Scope 树传播、在途副作用收敛与崩溃恢复。
//!
//! Cancelled 只表示执行安全收敛，不表示外部世界已经回滚。

use std::collections::{BTreeMap, BTreeSet};

use else_gateway::ExecContext;
use else_ledger::{IntentProjection, StoredEvent};
use else_protocol::attention::{Decision, DecisionChoice};
use else_protocol::effect::{EffectConclusion, ToolCall};
use else_protocol::event::{Actor, EventPayload, ScopeState};
use else_protocol::ids::{IntentId, RunId, ScopeId, ToolCallId};
use else_protocol::intent::IntentState;

use crate::scope::scope_spec;
use crate::{ApiError, Result, Runtime};

impl Runtime {
    /// 请求取消根执行及其全部后代。重复 Abort 与终态 Abort 都是幂等操作。
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
            .last()
            .ok_or_else(|| ApiError::IntentNotFound(intent_id.clone()))?;
        let root = self
            .latest_root_scope(intent_id)?
            .unwrap_or_else(|| source.envelope.scope_id.clone());
        let generation = projection
            .scopes
            .get(&root)
            .map(|scope| scope.generation)
            .unwrap_or(source.envelope.generation);
        let run_id = latest_run_id(&events, &root).unwrap_or_default();
        let ctx = ExecContext {
            spec,
            scope_id: root,
            run_id,
            generation,
            snapshot_id: None,
        };
        ctx.record(
            &self.ledger,
            Actor::User,
            EventPayload::ScopeAbortRequested {
                reason: reason.to_string(),
                scope_only: None,
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

    /// 补齐传播、逐 Scope 收敛，最后关闭 Decision 与根 Intent。每一步都可重入。
    /// scope_only 取消（Intent Revision 联动）只收敛子树，不关闭 Intent。
    pub(crate) fn recover_abort(&self, intent_id: &IntentId) -> Result<()> {
        let events = self.ledger.read_intent(intent_id)?;
        let Some(latest_abort_seq) = abort_events(&events).last().map(|event| event.seq) else {
            return Ok(());
        };
        if has_intent_cancelled_after(&events, latest_abort_seq) {
            return Ok(());
        }

        // 根取消事件先持久化；即使传播中途崩溃，也能按委托树补齐后代取消。
        self.propagate_abort(intent_id)?;
        let propagated = self.ledger.read_intent(intent_id)?;
        let aborts = abort_events(&propagated);
        for abort in aborts {
            self.recover_scope_abort(intent_id, abort.seq)?;
        }

        let events = self.ledger.read_intent(intent_id)?;
        let aborts = abort_events(&events);
        let intent_level: Vec<_> = aborts.iter().filter(|abort| !scope_only(abort)).collect();
        if intent_level.is_empty() {
            return Ok(());
        }
        let all_cancelled = aborts.iter().all(|abort| {
            has_scope_cancelled_after(
                &events,
                abort.seq,
                &abort.envelope.scope_id,
                abort.envelope.generation,
            )
        });
        if !all_cancelled {
            return Ok(());
        }

        let projection = self.projection(intent_id)?;
        let latest_abort = *intent_level
            .last()
            .ok_or_else(|| ApiError::NotFound("abort event".into()))?;
        let ctx = ExecContext {
            spec: scope_spec(&projection, &latest_abort.envelope.scope_id)
                .ok_or_else(|| ApiError::IntentNotFound(intent_id.clone()))?,
            scope_id: latest_abort.envelope.scope_id.clone(),
            run_id: latest_abort.envelope.run_id.clone().unwrap_or_default(),
            generation: latest_abort.envelope.generation,
            snapshot_id: None,
        };
        // 旧 Decision 一并失效，用户不能批准已经取消的父子执行。
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
        if !has_intent_cancelled_after(&events, latest_abort.seq) {
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

    /// 取消沿 Scope 树只向后代传播，权限与生命周期都不会反向扩大。
    fn propagate_abort(&self, intent_id: &IntentId) -> Result<()> {
        let events = self.ledger.read_intent(intent_id)?;
        let projection = self.projection(intent_id)?;
        let aborts = abort_events(&events);
        let mut requested: BTreeSet<(ScopeId, u64)> = aborts
            .iter()
            .map(|event| (event.envelope.scope_id.clone(), event.envelope.generation))
            .collect();
        for abort in aborts {
            for scope_id in descendants(&projection, &abort.envelope.scope_id) {
                let key = (scope_id.clone(), abort.envelope.generation);
                if requested.contains(&key) {
                    continue;
                }
                let Some(scope) = projection.scopes.get(&scope_id) else {
                    continue;
                };
                if scope.state != ScopeState::Running {
                    continue;
                }
                let ctx = ExecContext {
                    spec: scope_spec(&projection, &scope_id)
                        .ok_or_else(|| ApiError::IntentNotFound(intent_id.clone()))?,
                    scope_id: scope_id.clone(),
                    run_id: latest_run_id(&events, &scope_id).unwrap_or_default(),
                    generation: abort.envelope.generation,
                    snapshot_id: None,
                };
                ctx.record(
                    &self.ledger,
                    Actor::System,
                    EventPayload::ScopeAbortRequested {
                        reason: "父 Scope 取消，沿委托树传播".into(),
                        scope_only: match &abort.envelope.payload {
                            EventPayload::ScopeAbortRequested { scope_only, .. } => *scope_only,
                            _ => None,
                        },
                    },
                )?;
                requested.insert(key);
            }
        }
        Ok(())
    }

    fn recover_scope_abort(&self, intent_id: &IntentId, abort_seq: u64) -> Result<()> {
        let events = self.ledger.read_intent(intent_id)?;
        let abort = events
            .iter()
            .find(|event| event.seq == abort_seq)
            .ok_or_else(|| ApiError::NotFound(format!("abort event {abort_seq}")))?;
        let scope_id = abort.envelope.scope_id.clone();
        let generation = abort.envelope.generation;
        if has_scope_cancelled_after(&events, abort_seq, &scope_id, generation) {
            return Ok(());
        }
        let after_abort = |event: &&StoredEvent| {
            event.seq > abort_seq
                && event.envelope.scope_id == scope_id
                && event.envelope.generation == generation
        };
        let projection = self.projection(intent_id)?;
        let ctx = ExecContext {
            spec: scope_spec(&projection, &scope_id)
                .ok_or_else(|| ApiError::IntentNotFound(intent_id.clone()))?,
            scope_id: scope_id.clone(),
            run_id: abort.envelope.run_id.clone().unwrap_or_default(),
            generation,
            snapshot_id: None,
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

        self.revoke_active_leases(&ctx, &events)?;
        let mut started = BTreeSet::new();
        let mut finished = BTreeSet::new();
        for event in events
            .iter()
            .filter(|event| event.seq < abort_seq && event.envelope.scope_id == scope_id)
        {
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

        let inflight = inflight_before(&events, abort_seq, &scope_id);
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
        ctx.record(
            &self.ledger,
            Actor::System,
            EventPayload::ScopeStateChanged {
                state: ScopeState::Cancelled,
            },
        )?;
        Ok(())
    }

    /// 测试故障注入：持久化 N 条事件后模拟进程被 kill。
    #[doc(hidden)]
    pub fn crash_after_appends(&self, appends: u64) {
        self.ledger.crash_after_appends(appends);
    }
}

fn abort_events(events: &[StoredEvent]) -> Vec<&StoredEvent> {
    events
        .iter()
        .filter(|event| {
            matches!(
                event.envelope.payload,
                EventPayload::ScopeAbortRequested { .. }
            )
        })
        .collect()
}

/// scope_only 取消只收敛 Scope 子树（Intent Revision 联动），不关闭 Intent。
fn scope_only(event: &StoredEvent) -> bool {
    matches!(
        event.envelope.payload,
        EventPayload::ScopeAbortRequested {
            scope_only: Some(true),
            ..
        }
    )
}

fn descendants(projection: &IntentProjection, root: &ScopeId) -> BTreeSet<ScopeId> {
    let mut result = BTreeSet::new();
    loop {
        let mut changed = false;
        for (scope_id, scope) in &projection.scopes {
            if scope
                .parent
                .as_ref()
                .is_some_and(|parent| parent == root || result.contains(parent))
                && result.insert(scope_id.clone())
            {
                changed = true;
            }
        }
        if !changed {
            return result;
        }
    }
}

fn latest_run_id(events: &[StoredEvent], scope_id: &ScopeId) -> Option<RunId> {
    events.iter().rev().find_map(|event| {
        (event.envelope.scope_id == *scope_id)
            .then(|| event.envelope.run_id.clone())
            .flatten()
    })
}

fn has_scope_cancelled_after(
    events: &[StoredEvent],
    seq: u64,
    scope_id: &ScopeId,
    generation: u64,
) -> bool {
    events.iter().any(|event| {
        event.seq > seq
            && event.envelope.scope_id == *scope_id
            && event.envelope.generation == generation
            && matches!(
                event.envelope.payload,
                EventPayload::ScopeStateChanged {
                    state: ScopeState::Cancelled
                }
            )
    })
}

fn has_intent_cancelled_after(events: &[StoredEvent], seq: u64) -> bool {
    events.iter().any(|event| {
        event.seq > seq
            && matches!(
                event.envelope.payload,
                EventPayload::IntentStateChanged {
                    state: IntentState::Cancelled
                }
            )
    })
}

fn inflight_before(events: &[StoredEvent], abort_seq: u64, scope_id: &ScopeId) -> Vec<ToolCall> {
    let mut called: BTreeMap<ToolCallId, ToolCall> = BTreeMap::new();
    let mut done = BTreeSet::new();
    for event in events
        .iter()
        .filter(|event| event.seq < abort_seq && event.envelope.scope_id == *scope_id)
    {
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
