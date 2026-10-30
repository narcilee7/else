//! Harness 能力声明：不能支持的能力显式降级，不伪造事件。
//! 能力随 RunStarted 落账，让 Compare 能看见不同 Harness 的能力差异。

use serde::{Deserialize, Serialize};

/// 取消能力：优雅取消、只能整环境杀死（Opaque）、或不支持。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CancelSupport {
    /// 支持协作式取消（如 SIGINT 后收尾）。
    Cooperative,
    /// 不支持优雅取消：必须终止整个隔离环境并拒绝晚到写入。
    KillOnly,
}

/// 用量回报能力：真实数据、只有估算、或完全无法获得。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UsageSupport {
    Reported,
    Estimated,
    None,
}

/// 工具副作用的可见性：能过 Gateway、只能观察、或完全黑盒。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolEffectVisibility {
    /// 工具请求可交回 Tool Gateway 执行。
    GatewayRouted,
    /// Harness 自己执行工具，else 只能从输出流观察并落账。
    Observable,
    /// 黑盒：只知道有副作用，按 Unknown 保守处理并限制在 Sandbox 内。
    Opaque,
}

/// Harness 适配器能力契约（MVP-2）。随 RunStarted 落账，可重放、可比较。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct HarnessCapabilities {
    pub cancel: CancelSupport,
    /// 原生会话检查点/恢复能力；else 不依赖它做权威恢复。
    pub checkpoint_resume: bool,
    pub usage: UsageSupport,
    pub tool_effects: ToolEffectVisibility,
    /// 支持运行中追加输入（Decision 答案、后续指令）。
    pub interactive_input: bool,
}
