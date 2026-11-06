//! 副作用与工具调用。
//!
//! 不假装世界可以回滚：Abort Succeeded 只代表执行已经安全收敛，
//! 不代表世界恢复到了执行前。每个工具必须声明自己的副作用属性。

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::ids::ToolCallId;

/// 副作用等级，声明顺序即严格程度（派生 Ord）。
/// `Unknown` 最保守：默认只读或需要人工授权。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SideEffectKind {
    ReadOnly,
    Reversible,
    Compensatable,
    Irreversible,
    Unknown,
}

/// 一次工具调用。执行时由 Gateway 附加
/// `thread_id / scope_id / run_id / generation`（见 EventEnvelope）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: ToolCallId,
    pub tool: String,
    pub effect: SideEffectKind,
    pub input: Value,
    /// 幂等键：重试与晚到回调不得造成重复副作用。
    pub idempotency_key: String,
}

/// 副作用的最终结论。在途请求结果未知时必须先查询，不能盲目重试。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EffectConclusion {
    /// 未产生副作用。
    NoEffect,
    /// 副作用已发生且仍然成立。
    Applied,
    /// 已通过补偿动作收尾。
    Compensated,
    /// 结论未知，需要人工或后续核查。
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolStatus {
    Succeeded,
    Failed,
    /// 结果未知的在途请求。
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolOutcome {
    pub status: ToolStatus,
    pub output: Value,
    pub effect: EffectConclusion,
}
