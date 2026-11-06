//! Thread：用户目标及其边界。
//!
//! 核心不变量：Runtime 可以更新进度、计划和世界状态，
//! 但不能通过执行过程悄悄修改用户目标——`ThreadSpec` 只有用户
//! 或其明确授权的决策能改变，每次改变递增 `revision`。

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::connector::{ConnectorPolicy, ThreadOrigin};
use crate::effect::SideEffectKind;
use crate::ids::ThreadId;

/// 验收检查。Harness 报告完成只是候选结果，Verifier 必须独立执行检查。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum CriterionVerification {
    /// 无法自动验证，必须由用户确认。
    #[default]
    Manual,
    /// Workspace 内文件存在。
    FileExists { path: String },
    /// Workspace 内文件包含指定文本。
    FileContains { path: String, text: String },
}

/// 验收标准。Verifier 永远按原始 `done_when` 验收，不按最新 Plan。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Criterion {
    pub description: String,
    /// 缺省为 Manual，兼容旧事件；旧标准不会被静默视为已验证。
    #[serde(default)]
    pub verification: CriterionVerification,
}

impl Criterion {
    pub fn file_exists(description: impl Into<String>, path: impl Into<String>) -> Self {
        Self {
            description: description.into(),
            verification: CriterionVerification::FileExists { path: path.into() },
        }
    }

    pub fn manual(description: impl Into<String>) -> Self {
        Self {
            description: description.into(),
            verification: CriterionVerification::Manual,
        }
    }
}

/// 系统补全的假设。抽象表达由系统补全，但补全必须显式成为
/// Assumption，而不是变成隐蔽事实；用户确认后 `confirmed = true`。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Assumption {
    pub text: String,
    pub confirmed: bool,
}

/// 授权边界：可以替用户决定什么。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Authority {
    /// 可自主执行的最高副作用等级。`Unknown` 永远需要人工批准，
    /// 不允许出现在这里（由调用方在准入时校验）。
    pub max_autonomous_effect: SideEffectKind,
    /// 明确禁止的行为（自然语言或策略表达式），优先级高于一切授权。
    pub forbidden: Vec<String>,
}

impl Authority {
    /// 委托只能收窄：自主上限不得提高，父级禁令一条也不能丢。
    pub fn permits_child(&self, child: &Authority) -> bool {
        child.max_autonomous_effect <= self.max_autonomous_effect
            && self
                .forbidden
                .iter()
                .all(|rule| child.forbidden.contains(rule))
    }
}

/// 注意力契约：什么时候值得打断用户。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AttentionPolicy {
    /// 是否允许 Interrupt（立即打断）。false 时最高只到 Batch。
    pub allow_interrupt: bool,
    /// Batch 汇报的合并窗口（分钟）。
    pub batch_interval_minutes: u32,
}

/// 无法经过 Tool Gateway 的 Harness 需要用户显式开启 Opaque Profile。
/// 默认全关；Policy 不扩大 Thread Authority，只声明可观察风险的上限。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpaqueHarnessPolicy {
    pub enabled: bool,
    pub allow_native_provider_network: bool,
    #[serde(default)]
    pub use_tool_gateway: bool,
    pub max_observed_effect: SideEffectKind,
}

impl Default for OpaqueHarnessPolicy {
    fn default() -> Self {
        Self {
            enabled: false,
            allow_native_provider_network: false,
            use_tool_gateway: false,
            max_observed_effect: SideEffectKind::ReadOnly,
        }
    }
}

/// Thread 的**执行位置**。
///
/// else 是个人 Agent：一次工作可能是写周报、查资料、改代码、发邮件。
/// 把位置绑成 git worktree 会把产品缩成一种模式，所以位置是 Thread 的属性，
/// 不是全局设置——同一个 Runtime 里可以同时存在两种。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ThreadMode {
    /// 默认：Runtime 下的隔离目录，每个 Thread 一个，可以被安全遗弃。
    #[default]
    Scratch,
    /// 用户指定的真实目录：文档、资料库、代码仓库都行。
    /// Runtime 不拥有它、不复制它，删除 Thread 也不会删它。
    Directory { path: String },
}

impl ThreadMode {
    pub fn is_scratch(&self) -> bool {
        matches!(self, Self::Scratch)
    }

    /// 人话描述，用于 CLI/TUI 与错误信息。
    pub fn label(&self) -> String {
        match self {
            Self::Scratch => "scratch".into(),
            Self::Directory { path } => path.clone(),
        }
    }
}

/// Thread 的用户目标契约（ThreadSpec）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ThreadSpec {
    pub id: ThreadId,
    /// Thread Revision：目标语义的版本，每次用户变更递增。
    /// 与 Execution Generation（哪些执行与写入仍有效）是两个独立版本。
    pub revision: u64,
    /// 希望现实发生什么变化。
    pub outcome: String,
    pub done_when: Vec<Criterion>,
    /// 不能牺牲什么：质量、时间、稳定性、预算等。
    pub constraints: Vec<String>,
    pub authority: Authority,
    pub attention_policy: AttentionPolicy,
    /// 在哪里做。缺省是隔离目录，不假设目标是代码。
    #[serde(default)]
    pub mode: ThreadMode,
    /// Opaque Harness 的显式用户风险配置；缺省拒绝。
    #[serde(default)]
    pub opaque_harness_policy: OpaqueHarnessPolicy,
    pub deadline: Option<DateTime<Utc>>,
    pub assumptions: Vec<Assumption>,
    /// 项目作用域：Episodic/Semantic 记忆按项目召回。
    /// 缺省 None 只召回 Global。
    #[serde(default)]
    pub project: Option<String>,
    /// 来源血缘；旧事件缺省为未知，新 Thread 创建时显式记录。
    #[serde(default)]
    pub origin: Option<ThreadOrigin>,
    /// Connector Resolver 的受限自动接受策略；默认完全关闭。
    #[serde(default)]
    pub connector_policy: ConnectorPolicy,
}

/// ThreadState：由 Runtime 根据执行事件持续更新的投影状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThreadState {
    Active,
    WaitingExternal,
    NeedsDecision,
    Paused,
    Completed,
    Cancelled,
    Failed,
}
