//! Scope 失败分类、有限重试与终态收敛。

use else_gateway::{ExecContext, GatewayError};
use else_protocol::event::{Actor, EventPayload, ScopeState};
use else_protocol::failure::{FailureClass, ScopeFailure};

use crate::{ApiError, Result, Runtime};

const DEFAULT_MAX_SCOPE_RETRIES: u32 = 2;

impl Runtime {
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
                    EventPayload::IntentStateChanged {
                        state: else_protocol::intent::IntentState::Failed,
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
        | GatewayError::InvalidPath(_) => FailureClass::Terminal,
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
        GatewayError::Io(_) => "io",
        GatewayError::Timeout(_) => "timeout",
        GatewayError::Model(_) => "harness",
    }
}
