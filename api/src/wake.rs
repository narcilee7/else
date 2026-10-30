//! 外部事件与持久化 Timer：所有唤醒条件先成为事件，再参与 Kernel 裁决。

use chrono::{DateTime, Utc};
use else_ledger::IntentProjection;
use else_protocol::event::{Actor, EventEnvelope, EventPayload};
use else_protocol::ids::{IntentId, ScopeId, TimerId};
use else_protocol::intent::IntentState;

use crate::{ApiError, Result, Runtime};

impl Runtime {
    pub fn record_external_event(
        &self,
        intent_id: &IntentId,
        source: &str,
        summary: &str,
    ) -> Result<IntentProjection> {
        let (scope_id, generation, revision) = self.kernel_context(intent_id)?;
        self.ledger.append(&EventEnvelope::new(
            intent_id.clone(),
            scope_id.clone(),
            generation,
            revision,
            Actor::External,
            EventPayload::ExternalEventReceived {
                source: source.to_string(),
                summary: summary.to_string(),
            },
        ))?;
        let projection = self.projection(intent_id)?;
        if matches!(
            projection.state,
            Some(IntentState::WaitingExternal | IntentState::Paused)
        ) {
            self.ledger.append(&EventEnvelope::new(
                intent_id.clone(),
                scope_id,
                generation,
                revision,
                Actor::System,
                EventPayload::IntentStateChanged {
                    state: IntentState::Active,
                },
            ))?;
        }
        self.projection(intent_id)
    }

    pub fn schedule_timer(
        &self,
        intent_id: &IntentId,
        fire_at: DateTime<Utc>,
        reason: &str,
    ) -> Result<TimerId> {
        let (scope_id, generation, revision) = self.kernel_context(intent_id)?;
        let timer_id = TimerId::new();
        self.ledger.append(&EventEnvelope::new(
            intent_id.clone(),
            scope_id,
            generation,
            revision,
            Actor::System,
            EventPayload::TimerScheduled {
                timer_id: timer_id.clone(),
                fire_at,
                reason: reason.to_string(),
            },
        ))?;
        Ok(timer_id)
    }

    pub fn cancel_timer(
        &self,
        intent_id: &IntentId,
        timer_id: &TimerId,
        reason: &str,
    ) -> Result<()> {
        let (scope_id, generation, revision) = self.kernel_context(intent_id)?;
        self.ledger.append(&EventEnvelope::new(
            intent_id.clone(),
            scope_id,
            generation,
            revision,
            Actor::System,
            EventPayload::TimerCancelled {
                timer_id: timer_id.clone(),
                reason: reason.to_string(),
            },
        ))?;
        Ok(())
    }

    pub(crate) fn fire_timer(&self, intent_id: &IntentId, timer_id: &TimerId) -> Result<()> {
        let projection = self.projection(intent_id)?;
        let timer = projection
            .timers
            .get(timer_id)
            .ok_or_else(|| ApiError::NotFound(format!("timer {timer_id}")))?;
        if timer.fire_at > Utc::now() {
            return Err(ApiError::Conflict(format!(
                "timer {timer_id} is not due until {}",
                timer.fire_at
            )));
        }
        let (scope_id, generation, revision) = self.kernel_context(intent_id)?;
        let envelope = |payload| {
            EventEnvelope::new(
                intent_id.clone(),
                scope_id.clone(),
                generation,
                revision,
                Actor::System,
                payload,
            )
        };
        self.ledger.append(&envelope(EventPayload::TimerFired {
            timer_id: timer_id.clone(),
        }))?;
        Ok(())
    }

    fn kernel_context(&self, intent_id: &IntentId) -> Result<(ScopeId, u64, u64)> {
        let projection = self.projection(intent_id)?;
        let spec = projection
            .spec
            .ok_or_else(|| ApiError::IntentNotFound(intent_id.clone()))?;
        let scope_id = projection
            .root_scope_id
            .ok_or_else(|| ApiError::IntentNotFound(intent_id.clone()))?;
        let generation = projection
            .scopes
            .get(&scope_id)
            .map(|scope| scope.generation)
            .unwrap_or(1);
        Ok((scope_id, generation, spec.revision))
    }
}
