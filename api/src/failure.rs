//! Scope 失败分类、有限重试与终态收敛。

use else_gateway::{ExecContext, GatewayError};
use else_protocol::event::{Actor, EventEnvelope, EventPayload, ScopeState};
use else_protocol::failure::{FailureClass, ScopeFailure};
use else_protocol::ids::{ScopeId, ThreadId};
use else_protocol::thread::ThreadState;

use crate::{ApiError, Result, Runtime};

const DEFAULT_MAX_SCOPE_RETRIES: u32 = 2;

impl Runtime {
    pub fn retry_failed_scopes(
        &self,
        thread_id: &ThreadId,
    ) -> Result<(Vec<ScopeId>, else_ledger::ThreadProjection)> {
        let projection = self.projection(thread_id)?;
        let revision = projection
            .spec
            .as_ref()
            .ok_or_else(|| ApiError::ThreadNotFound(thread_id.clone()))?
            .revision;
        let failed = projection
            .scopes
            .iter()
            .filter(|(_, scope)| scope.state == ScopeState::Failed)
            .map(|(scope_id, _)| scope_id.clone())
            .collect::<Vec<_>>();
        if failed.is_empty() {
            return Err(ApiError::Conflict(
                "thread has no failed scopes to retry".into(),
            ));
        }
        let generation = self
            .events(thread_id)?
            .iter()
            .map(|event| event.envelope.generation)
            .max()
            .unwrap_or(1)
            .saturating_add(1);
        for scope_id in &failed {
            self.ledger.append(&EventEnvelope::new(
                thread_id.clone(),
                scope_id.clone(),
                generation,
                revision,
                Actor::User,
                EventPayload::ScopeRetryApproved,
            ))?;
        }
        if projection
            .root_scope_id
            .as_ref()
            .is_some_and(|root| failed.contains(root))
        {
            self.ledger.append(&EventEnvelope::new(
                thread_id.clone(),
                projection.root_scope_id.clone().unwrap(),
                generation,
                revision,
                Actor::User,
                EventPayload::ThreadStateChanged {
                    state: ThreadState::Active,
                },
            ))?;
        }
        let _ = self.drive(thread_id, None, 16)?;
        let projection = self.projection(thread_id)?;
        Ok((failed, projection))
    }

    /// Admission 发生在 RunStarted 之前；失败也必须落 ScopeFailure，
    /// 否则依赖会永久等待一个永远不会启动的上游。
    pub(crate) fn admit_run_with_failure(
        &self,
        ctx: &ExecContext,
    ) -> Result<else_protocol::resource::ResourceLease> {
        match self.admit_run(ctx) {
            Ok(lease) => Ok(lease),
            Err(ApiError::Gateway(error)) => {
                self.record_scope_failure(ctx, &error)?;
                Err(ApiError::Gateway(error))
            }
            Err(error) => Err(error),
        }
    }

    /// 记录失败并决定是否继续自动重试。达到重试上限或终态故障时，
    /// Scope 才进入 Failed；可重试故障保持 Running 供 Kernel 生成下一代。
    pub(crate) fn record_scope_failure(
        &self,
        ctx: &ExecContext,
        error: &GatewayError,
    ) -> Result<()> {
        if matches!(
            error,
            GatewayError::Ledger(else_ledger::LedgerError::CrashInjected)
                | GatewayError::GenerationFenced { .. }
        ) {
            return Ok(());
        }
        let projection = self.projection(&ctx.spec.id)?;
        let scope = projection
            .scopes
            .get(&ctx.scope_id)
            .ok_or_else(|| ApiError::NotFound(format!("scope {}", ctx.scope_id)))?;
        if !matches!(scope.state, ScopeState::Running | ScopeState::Paused) {
            return Ok(());
        }
        let class = classify_failure(error);
        let attempt = scope.failure_count.saturating_add(1);
        let max_retries = std::env::var("ELSE_MAX_SCOPE_RETRIES")
            .ok()
            .and_then(|raw| raw.parse().ok())
            .unwrap_or(DEFAULT_MAX_SCOPE_RETRIES);
        ctx.record(
            &self.ledger,
            Actor::System,
            EventPayload::ScopeFailureRecorded {
                failure: ScopeFailure {
                    run_id: ctx.run_id.clone(),
                    generation: ctx.generation,
                    class,
                    code: failure_code(error).into(),
                    message: error.to_string(),
                    attempt,
                },
            },
        )?;
        if class == FailureClass::Terminal || attempt > max_retries {
            ctx.record(
                &self.ledger,
                Actor::System,
                EventPayload::ScopeStateChanged {
                    state: ScopeState::Failed,
                },
            )?;
            if projection.root_scope_id.as_ref() == Some(&ctx.scope_id) {
                ctx.record(
                    &self.ledger,
                    Actor::System,
                    EventPayload::ThreadStateChanged {
                        state: else_protocol::thread::ThreadState::Failed,
                    },
                )?;
            }
        }
        Ok(())
    }
}

fn classify_failure(error: &GatewayError) -> FailureClass {
    match error {
        GatewayError::Timeout(_) | GatewayError::Model(_) | GatewayError::Io(_) => {
            FailureClass::Retryable
        }
        GatewayError::NeedsApproval(_) => FailureClass::Terminal,
        GatewayError::Ledger(_)
        | GatewayError::GenerationFenced { .. }
        | GatewayError::LeaseRequired(_)
        | GatewayError::ResourceExhausted { .. }
        | GatewayError::UnknownTool(_)
        | GatewayError::InvalidPath(_)
        | GatewayError::OpaqueHarnessDenied(_)
        | GatewayError::OpaqueEffectDenied(_) => FailureClass::Terminal,
    }
}

fn failure_code(error: &GatewayError) -> &'static str {
    match error {
        GatewayError::Ledger(_) => "ledger",
        GatewayError::NeedsApproval(_) => "needs_approval",
        GatewayError::GenerationFenced { .. } => "generation_fenced",
        GatewayError::LeaseRequired(_) => "lease_required",
        GatewayError::ResourceExhausted { .. } => "resource_exhausted",
        GatewayError::UnknownTool(_) => "unknown_tool",
        GatewayError::InvalidPath(_) => "invalid_path",
        GatewayError::OpaqueHarnessDenied(_) => "opaque_harness_denied",
        GatewayError::OpaqueEffectDenied(_) => "opaque_effect_denied",
        GatewayError::Io(_) => "io",
        GatewayError::Timeout(_) => "timeout",
        GatewayError::Model(_) => "harness",
    }
}
