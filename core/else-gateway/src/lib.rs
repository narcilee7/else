//! else 网关：信任边界。
//!
//! 策略自研、数据面复用。Agent 不拥有权限：
//! - 所有副作用经过 Tool Gateway（授权检查、幂等、事件落账）；
//! - 所有模型调用经过 Model Gateway（Token/费用入账）。
//!
//! W2 薄版：文件/shell 两工具、直连 Provider；Generation Fencing、
//! Lease 校验、配额熔断在 M1 补齐。

pub mod model;
pub mod tool;

pub use model::{
    MockProvider, ModelGateway, ModelProvider, ModelRequest, ModelResponse, OpenAiCompatProvider,
};
pub use tool::ToolGateway;

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

#[derive(Debug, thiserror::Error)]
pub enum GatewayError {
    #[error("ledger: {0}")]
    Ledger(#[from] else_ledger::LedgerError),
    /// 超出授权边界：已提交 AttentionRequest，等待本人决策。
    #[error("needs human approval: {0}")]
    NeedsApproval(AttentionId),
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
