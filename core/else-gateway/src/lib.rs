//! else 网关：信任边界。
//!
//! 策略自研、数据面复用。Agent 不拥有权限：
//! - 所有副作用经过 Tool Gateway（授权检查、幂等、事件落账）；
//! - 所有模型调用经过 Model Gateway（Token/费用入账）。
//!
//! 当前：文件/shell 工具、genai Provider、Generation Fencing、Lease 与配额熔断。

pub mod model;
pub mod resource;
pub mod tool;

pub use model::{
    GenAiProvider, MockProvider, ModelGateway, ModelProvider, ModelRequest, ModelResponse,
};
pub use tool::ToolGateway;

use else_ledger::Ledger;
use else_protocol::event::EventPayload;
use else_protocol::ids::{AttentionId, RunId, ScopeId};
use else_protocol::intent::IntentSpec;

/// 一次执行的上下文：授权依据（IntentSpec）与执行定位。
/// generation 在 M1 用于围栏旧世代的写入。
#[derive(Debug, Clone)]
pub struct ExecContext {
    pub spec: IntentSpec,
    pub scope_id: ScopeId,
    pub run_id: RunId,
    pub generation: u64,
}

impl ExecContext {
    /// 以当前执行上下文落一条事件（自动携带 generation/revision/run_id）。
    pub fn record(
        &self,
        ledger: &else_ledger::Ledger,
        actor: else_protocol::event::Actor,
        payload: else_protocol::event::EventPayload,
    ) -> std::result::Result<else_ledger::StoredEvent, else_ledger::LedgerError> {
        let mut envelope = else_protocol::event::EventEnvelope::new(
            self.spec.id.clone(),
            self.scope_id.clone(),
            self.generation,
            self.spec.revision,
            actor,
            payload,
        );
        envelope.run_id = Some(self.run_id.clone());
        ledger.append(&envelope)
    }
}

/// Generation Fencing：旧世代与已请求 Abort 的世代不得再产生执行事件或外部动作。
/// 检查放在 Gateway 入口与完成回调落账前，覆盖晚到回调。
pub fn ensure_not_fenced(ledger: &Ledger, ctx: &ExecContext) -> Result<()> {
    let events = ledger.read_intent(&ctx.spec.id)?;
    let latest_generation = events
        .iter()
        .map(|event| event.envelope.generation)
        .max()
        .unwrap_or(ctx.generation);
    let aborted = events.iter().any(|event| {
        event.envelope.generation == ctx.generation
            && matches!(
                event.envelope.payload,
                EventPayload::ScopeAbortRequested { .. }
            )
    });
    if aborted || ctx.generation < latest_generation {
        return Err(GatewayError::GenerationFenced {
            generation: ctx.generation,
        });
    }
    Ok(())
}

#[derive(Debug, thiserror::Error)]
pub enum GatewayError {
    #[error("ledger: {0}")]
    Ledger(#[from] else_ledger::LedgerError),
    /// 超出授权边界：已提交 AttentionRequest，等待本人决策。
    #[error("needs human approval: {0}")]
    NeedsApproval(AttentionId),
    /// 旧 Generation 或已 Abort 的 Generation 被围栏。
    #[error("generation fenced: {generation}")]
    GenerationFenced { generation: u64 },
    #[error("active resource lease required: {0}")]
    LeaseRequired(String),
    #[error("resource exhausted: {resource}, requested {requested} {unit}, remaining {remaining} {unit}")]
    ResourceExhausted {
        resource: String,
        requested: u64,
        remaining: u64,
        unit: &'static str,
    },
    #[error("unknown tool: {0}")]
    UnknownTool(String),
    #[error("invalid path (must stay inside workspace): {0}")]
    InvalidPath(String),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("model provider: {0}")]
    Model(String),
}

pub type Result<T> = std::result::Result<T, GatewayError>;
