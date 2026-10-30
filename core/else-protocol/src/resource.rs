//! 资源：统一 Admission + Lease + Accounting。
//!
//! 模型也是资源。CPU、Sandbox、API 限流、模型 Token/费用、
//! 用户注意力都走同一套租约；所有后代消耗同时计入子节点和根 Intent。

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::ids::{LeaseId, RunId, ScopeId};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResourceClass {
    Cpu,
    Memory,
    Disk,
    /// Sandbox 数量与执行时长。
    Sandbox,
    BrowserSession,
    /// 第三方账号 / API Rate Limit。
    ApiRateLimit,
    /// 工作区、文件、数据库记录的独占权。
    WorkspaceLock,
    ModelTokens,
    ModelSpend,
    ModelConcurrency,
    /// 用户的时间与注意力预算。
    HumanAttention,
}

/// 每个 Intent 的硬预算。预算随事件持久化，重启后 Admission 使用同一契约。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceBudget {
    pub max_model_tokens: u64,
    pub max_model_spend_usd_micros: u64,
    pub max_tool_calls: u64,
    pub max_sandbox_seconds: u64,
}

impl Default for ResourceBudget {
    fn default() -> Self {
        Self {
            max_model_tokens: 1_000_000,
            max_model_spend_usd_micros: 1_000_000,
            max_tool_calls: 10_000,
            max_sandbox_seconds: 86_400,
        }
    }
}

/// 从事件累计的资源账；后代事件自然计入根 Intent。
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceUsage {
    pub model_tokens: u64,
    pub model_spend_usd_micros: u64,
    pub tool_calls: u64,
    pub sandbox_seconds: u64,
}

/// 资源租约：资源不足就不允许启动（Admission），
/// 而不是启动后无限超卖。Lease 到期必须续租或回收。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResourceLease {
    pub id: LeaseId,
    pub scope_id: ScopeId,
    pub run_id: Option<RunId>,
    pub class: ResourceClass,
    /// 授予数量与单位（如 millicores / tokens / usd_micros / seconds）。
    pub quantity: u64,
    pub unit: String,
    /// 与执行世代绑定：Generation 失效后租约随之失效。
    pub generation: u64,
    pub expires_at: DateTime<Utc>,
}

/// 用量证据等级：真实数据记 Reported，估算记 Estimated，
/// 无法获得记 Unknown 并采用保守预算，不能记成零成本。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UsageBasis {
    /// 来自 Provider / Harness 回报的真实数据；缺省值兼容旧事件（网关实测）。
    #[default]
    Reported,
    Estimated,
    Unknown,
}

/// 一次模型调用的资源消耗（Accounting）。
/// 所有后代消耗同时计入子节点和根 Intent。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cost_usd_micros: u64,
    /// 用量证据等级；Unknown 时按保守预算记账，不视为零成本。
    #[serde(default)]
    pub basis: UsageBasis,
}

impl ModelUsage {
    /// Harness 无法回报用量时的保守估计：按一次完整响应记账。
    pub fn unknown() -> Self {
        Self {
            input_tokens: 4_096,
            output_tokens: 4_096,
            cost_usd_micros: 0,
            basis: UsageBasis::Unknown,
        }
    }
}
