//! 注意力协议：AttentionRequest / Ruling / Decision。
//!
//! 所有 Agent、连接器、定时器都不能直接通知用户，只能提交
//! AttentionRequest。判定原则：只有"等待或自主行动的预期损失"
//! 显著高于打断成本，且现有授权无法解决时，才允许 Interrupt。

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::event::Actor;
use crate::ids::{AttentionId, AttentionWindowId, TimerId};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttentionKind {
    /// 需要本人判断。
    Judgment,
    /// 需要授权（如不可逆操作、对外发送）。
    Authorization,
    /// 需要补充信息。
    MissingInfo,
    /// 需要本人接管。
    Takeover,
    /// 需要本人确认无法机器验证的 Outcome Evidence。
    Verification,
}

/// 不处理（等待或自主行动）的预期损失，与打断成本比较。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExpectedLoss {
    Low,
    Medium,
    High,
    Critical,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DecisionOption {
    pub id: String,
    pub label: String,
    pub recommended: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AttentionRequest {
    pub id: AttentionId,
    pub kind: AttentionKind,
    /// 需要人做什么（一句话）。
    pub summary: String,
    /// 为什么无法自行解决。
    pub blocked_reason: String,
    pub expected_loss: ExpectedLoss,
    /// 是否阻塞整个 Thread。
    pub blocking: bool,
    /// 等待是否可逆（拖久了是否还能补救）。
    pub reversible: bool,
    /// 最迟决策时间。
    pub decide_by: Option<DateTime<Utc>>,
    /// 推荐选项及默认动作。
    pub options: Vec<DecisionOption>,
    pub default_option: Option<String>,
    /// 关联的工具调用幂等键：本人批准后，Gateway 对同一键的调用放行
    /// （一次性授权，不扩大全局权限）。旧事件无此字段，反序列化为 None。
    #[serde(default)]
    pub related_call_key: Option<String>,
}

/// 从事件投影计算的注意力成本指标。误打断率只统计用户明确 Dismiss 的
/// Interrupt 请求，不用拒绝选项猜测。
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct AttentionMetrics {
    pub requested: u64,
    pub resolved: u64,
    pub interventions: u64,
    pub batch_notifications: u64,
    pub defer_notifications: u64,
    pub interrupt_notifications: u64,
    pub batched_requests: u64,
    pub deferred_requests: u64,
    pub silent_requests: u64,
    pub open_critical: u64,
    pub missed_critical: u64,
    pub resolved_interrupts: u64,
    pub dismissed_interrupts: u64,
    pub batch_rate: Option<f64>,
    pub misinterrupt_rate: Option<f64>,
}

/// Attention Kernel 的四种裁决。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "ruling", rename_all = "snake_case")]
pub enum Ruling {
    /// 无需惊动用户。
    Silent,
    /// 等到合适的专注间隙。
    Defer { until: DateTime<Utc> },
    /// 合并进下一次决策批次。
    Batch,
    /// 立即打断。
    Interrupt,
}

/// Attention Window 的调度类型。Batch 聚合多个请求，Defer 只延迟一个请求；
/// 两者都必须在持久窗口关闭后才能进入 Decision Queue。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttentionWindowKind {
    Batch,
    Defer,
}

/// 由事件重放得到的注意力窗口。`timer_id` 随窗口持久化，重启后可用同一 ID
/// 补齐丢失的唤醒事件，而不会生成第二次通知。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AttentionWindow {
    pub id: AttentionWindowId,
    pub kind: AttentionWindowKind,
    pub opened_at: DateTime<Utc>,
    pub due_at: DateTime<Utc>,
    pub timer_id: TimerId,
    pub closed_at: Option<DateTime<Utc>>,
    /// 只有截止时仍有未决成员的窗口才算形成一次实际通知批次。
    pub delivered_at: Option<DateTime<Utc>>,
    pub request_ids: Vec<AttentionId>,
}

impl AttentionWindow {
    pub fn is_closed(&self) -> bool {
        self.closed_at.is_some()
    }

    pub fn contains(&self, request_id: &AttentionId) -> bool {
        self.request_ids.contains(request_id)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum DecisionChoice {
    /// 选择了某个推荐选项（option id）。
    Option(String),
    /// 自由文本回答。
    FreeText(String),
    /// 用户驳回该请求（按默认或放弃路径处理）。
    Dismissed,
}

/// 本人的决定。Decision 是事件，不是可原地修改的状态。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Decision {
    pub request_id: AttentionId,
    pub choice: DecisionChoice,
    pub rationale: Option<String>,
    pub decided_by: Actor,
}
