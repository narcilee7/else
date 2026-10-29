//! Model Gateway：模型是资源，调用必须入账。
//!
//! 策略面自研（入账、授权、配额），数据面复用：
//! - 进程内 Provider 适配：genai（rust-genai），不手写协议适配；
//! - 独立网关（M3）：TensorZero / LiteLLM 等现成数据面。
//!
//! 当前已接入 Lease、配额与真实用量复核；降级路由在 M3 接入。

use else_ledger::Ledger;
use else_protocol::event::{Actor, EventPayload};
use else_protocol::resource::ModelUsage;

use crate::{ensure_not_fenced, ExecContext, GatewayError, Result};

#[derive(Debug, Clone)]
pub struct ModelRequest {
    pub model: String,
    pub prompt: String,
}

#[derive(Debug, Clone)]
pub struct ModelResponse {
    pub text: String,
    pub usage: ModelUsage,
}

/// Provider 适配层。实现必须幂等可读：调用失败返回错误，由上层决定重试。
pub trait ModelProvider: Send + Sync {
    fn complete(&self, request: &ModelRequest) -> std::result::Result<ModelResponse, String>;

    /// 调用前资源预估；真实用量返回后会再次校验并覆盖入账。
    fn estimate(&self, request: &ModelRequest) -> ModelUsage {
        ModelUsage {
            input_tokens: request.prompt.len() as u64 / 2,
            output_tokens: 64,
            cost_usd_micros: 0,
        }
    }
}

/// 确定性 Mock：本地演示与测试用，不产生真实外部调用。
pub struct MockProvider;

impl ModelProvider for MockProvider {
    fn complete(&self, request: &ModelRequest) -> std::result::Result<ModelResponse, String> {
        Ok(ModelResponse {
            text: format!(
                "1. 解析目标与约束\n2. 生成计划并写入工作区\n3. 校验产物后请求验收\n\n目标：{}",
                request.prompt
            ),
            usage: self.estimate(request),
        })
    }

    fn estimate(&self, request: &ModelRequest) -> ModelUsage {
        ModelUsage {
            input_tokens: request.prompt.len() as u64 / 2,
            output_tokens: 64,
            cost_usd_micros: 1_500,
        }
    }
}

/// 多 Provider 适配：复用 genai（rust-genai），不手写协议适配。
/// 支持 OpenAI/Anthropic/Gemini/Ollama/Moonshot 等 27+ Provider 原生协议；
/// 密钥走各 Provider 标准环境变量（OPENAI_API_KEY、ANTHROPIC_API_KEY 等）。
/// M1 引入 tokio 全链路异步后，此处的同步桥接随之移除。
pub struct GenAiProvider {
    client: genai::Client,
    runtime: tokio::runtime::Runtime,
}

impl GenAiProvider {
    pub fn new() -> std::result::Result<Self, String> {
        let client = genai::Client::new().map_err(|e| e.to_string())?;
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| e.to_string())?;
        Ok(Self { client, runtime })
    }
}

impl ModelProvider for GenAiProvider {
    fn complete(&self, request: &ModelRequest) -> std::result::Result<ModelResponse, String> {
        let chat_req =
            genai::chat::ChatRequest::new(vec![genai::chat::ChatMessage::user(&request.prompt)]);
        let response = self
            .runtime
            .block_on(self.client.exec_chat(&request.model, chat_req, None))
            .map_err(|e| e.to_string())?;
        Ok(ModelResponse {
            text: response.first_text().unwrap_or_default().to_string(),
            usage: ModelUsage {
                input_tokens: response.usage.prompt_tokens.unwrap_or(0) as u64,
                output_tokens: response.usage.completion_tokens.unwrap_or(0) as u64,
                cost_usd_micros: 0,
            },
        })
    }
}

pub struct ModelGateway<'a> {
    ledger: &'a Ledger,
    provider: Box<dyn ModelProvider>,
}

impl<'a> ModelGateway<'a> {
    pub fn new(ledger: &'a Ledger, provider: Box<dyn ModelProvider>) -> Self {
        Self { ledger, provider }
    }

    /// 调用并入账：围栏 + Lease + 预估 → Provider → 实际用量复核 → ModelCompleted。
    pub fn complete(
        &self,
        ctx: &ExecContext,
        purpose: &str,
        model: &str,
        prompt: &str,
    ) -> Result<ModelResponse> {
        ensure_not_fenced(self.ledger, ctx)?;
        crate::resource::ensure_active_lease(self.ledger, ctx)?;
        let request = ModelRequest {
            model: model.to_string(),
            prompt: prompt.to_string(),
        };
        crate::resource::ensure_model_capacity(self.ledger, ctx, self.provider.estimate(&request))?;
        ctx.record(
            self.ledger,
            Actor::Agent,
            EventPayload::ModelCalled {
                model: model.to_string(),
                purpose: purpose.to_string(),
            },
        )?;
        let response = self
            .provider
            .complete(&request)
            .map_err(GatewayError::Model)?;
        ensure_not_fenced(self.ledger, ctx)?;
        crate::resource::ensure_active_lease(self.ledger, ctx)?;
        crate::resource::ensure_model_capacity(self.ledger, ctx, response.usage)?;
        ctx.record(
            self.ledger,
            Actor::System,
            EventPayload::ModelCompleted {
                model: model.to_string(),
                usage: response.usage,
            },
        )?;
        Ok(response)
    }
}
