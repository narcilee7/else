//! Model Gateway：模型是资源，调用必须入账。
//!
//! W2 薄版：直连 Provider（OpenAI 兼容端点）或 Mock。
//! 配额、Lease、降级路由在 M1/M3 接入；Provider 适配层届时可替换为
//! LiteLLM Proxy / OpenRouter 等现成数据面，策略面保持不变。

use else_ledger::Ledger;
use else_protocol::event::{Actor, EventPayload};
use else_protocol::resource::ModelUsage;
use serde_json::{json, Value};

use crate::{ExecContext, GatewayError, Result};

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
            usage: ModelUsage {
                input_tokens: request.prompt.len() as u64 / 2,
                output_tokens: 64,
                cost_usd_micros: 1_500,
            },
        })
    }
}

/// OpenAI 兼容端点（/chat/completions）。成本单价表后置，先记录 Token。
pub struct OpenAiCompatProvider {
    pub base_url: String,
    pub api_key: String,
}

impl ModelProvider for OpenAiCompatProvider {
    fn complete(&self, request: &ModelRequest) -> std::result::Result<ModelResponse, String> {
        let body: Value = ureq::post(&format!("{}/chat/completions", self.base_url))
            .set("Authorization", &format!("Bearer {}", self.api_key))
            .send_json(json!({
                "model": request.model,
                "messages": [{"role": "user", "content": request.prompt}],
            }))
            .map_err(|e| e.to_string())?
            .into_json()
            .map_err(|e| e.to_string())?;
        let text = body["choices"][0]["message"]["content"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        Ok(ModelResponse {
            text,
            usage: ModelUsage {
                input_tokens: body["usage"]["prompt_tokens"].as_u64().unwrap_or(0),
                output_tokens: body["usage"]["completion_tokens"].as_u64().unwrap_or(0),
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

    /// 调用并入账：ModelCalled → Provider → ModelCompleted（Token/费用）。
    pub fn complete(
        &self,
        ctx: &ExecContext,
        purpose: &str,
        model: &str,
        prompt: &str,
    ) -> Result<ModelResponse> {
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
            .complete(&ModelRequest {
                model: model.to_string(),
                prompt: prompt.to_string(),
            })
            .map_err(GatewayError::Model)?;
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
