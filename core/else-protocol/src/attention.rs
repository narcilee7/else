//! 注意力协议：AttentionRequest / Ruling / Decision。
//!
//! 所有 Agent、连接器、定时器都不能直接通知用户，只能提交
//! AttentionRequest。判定原则：只有"等待或自主行动的预期损失"
//! 显著高于打断成本，且现有授权无法解决时，才允许 Interrupt。

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::event::Actor;
use crate::ids::AttentionId;

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
    /// 是否阻塞整个 Intent。
    pub blocking: bool,
    /// 等待是否可逆（拖久了是否还能补救）。
    pub reversible: bool,
    /// 最迟决策时间。
    pub decide_by: Option<DateTime<Utc>>,
    /// 推荐选项及默认动作。
    pub options: Vec<DecisionOption>,
    pub default_option: Option<String>,
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
