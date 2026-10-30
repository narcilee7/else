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
    /// 已消费的 Sandbox 墙钟秒数（Reported + Estimated + Unknown）。
    pub sandbox_seconds: u64,
    /// 当前仍活跃 Lease 的保守预留秒数，不是实际成本。
    #[serde(default)]
    pub sandbox_reserved_seconds: u64,
    #[serde(default)]
    pub sandbox_reported_seconds: u64,
    #[serde(default)]
    pub sandbox_estimated_seconds: u64,
    #[serde(default)]
    pub sandbox_unknown_seconds: u64,
    #[serde(default)]
    pub sandbox_actual_millis: u64,
}

impl ResourceUsage {
    /// Admission 使用：已消费实际 + 当前预留，避免活跃 Run 被重复超卖。
    pub fn sandbox_committed_seconds(&self) -> u64 {
        self.sandbox_seconds
            .saturating_add(self.sandbox_reserved_seconds)
    }
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
    /// Lease 签发时间；旧事件缺省，由 `LeaseGranted.occurred_at` 恢复。
    #[serde(default)]
    pub granted_at: Option<DateTime<Utc>>,
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SandboxUsageBasis {
    Reported,
    Estimated,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SandboxTerminationReason {
    Completed,
    Failed,
    TimedOut,
    PausedForDecision,
    Aborted,
    Crashed,
    Unknown,
}

/// 一次 Sandbox 生命周期的实际墙钟与证据。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SandboxUsage {
    pub lease_id: LeaseId,
    pub backend: String,
    pub started_at: DateTime<Utc>,
    pub finished_at: DateTime<Utc>,
    pub elapsed_millis: u64,
    pub basis: SandboxUsageBasis,
    pub termination: SandboxTerminationReason,
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
