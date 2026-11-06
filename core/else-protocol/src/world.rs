//! Personal World：从 Signal Interpretation 聚合出的承诺、等待与行动状态。

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::connector::{CorrelationHint, CorrelationKind, ExternalEntityRef, ObjectKind};
use crate::ids::{ConnectorId, ThreadId, WorldCorrelationCorrectionId, WorldItemId, WorldSignalId};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorldItemKind {
    Commitment,
    WaitingExternal,
    ActionItem,
    Risk,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorldItemState {
    Open,
    WaitingExternal,
    Overdue,
    AtRisk,
    Completed,
    Dismissed,
    Superseded,
}

impl WorldItemState {
    pub fn is_resolved(self) -> bool {
        matches!(self, Self::Completed | Self::Dismissed | Self::Superseded)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorldItem {
    pub id: WorldItemId,
    pub entity_ref: ExternalEntityRef,
    pub object_kind: ObjectKind,
    pub kind: WorldItemKind,
    pub state: WorldItemState,
    pub title: String,
    pub summary: String,
    pub deadline: Option<DateTime<Utc>>,
    /// 0-100，由状态、语义、期限和解释置信度确定性计算。
    #[serde(default)]
    pub priority: u8,
    /// 面向用户的可读排序依据，不伪装成模型主观判断。
    #[serde(default)]
    pub priority_reason: String,
    pub signal_ids: Vec<WorldSignalId>,
    pub connector_ids: Vec<ConnectorId>,
    #[serde(default)]
    pub projects: Vec<CorrelationHint>,
    /// 联系人、客户、线程与 Milestone 等非 Project 身份关联。
    #[serde(default)]
    pub identities: Vec<CorrelationHint>,
    pub first_observed_at: DateTime<Utc>,
    pub last_observed_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorldItemRecord {
    #[serde(alias = "intent_id")]
    pub thread_id: ThreadId,
    pub item: WorldItem,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProjectWorld {
    pub key: String,
    pub title: String,
    pub items: Vec<WorldItemRecord>,
    #[serde(alias = "intent_ids")]
    pub thread_ids: Vec<ThreadId>,
    pub connector_ids: Vec<ConnectorId>,
    pub open_items: u64,
    pub waiting_external: u64,
    pub overdue: u64,
    pub next_deadline: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IdentityWorld {
    pub kind: CorrelationKind,
    pub key: String,
    pub title: String,
    pub items: Vec<WorldItemRecord>,
    #[serde(alias = "intent_ids")]
    pub thread_ids: Vec<ThreadId>,
    pub connector_ids: Vec<ConnectorId>,
    pub open_items: u64,
    pub overdue: u64,
    pub next_deadline: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorldItemResolutionKind {
    Completed,
    Dismissed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorldCorrelationCorrectionKind {
    Merge,
    Split,
    Rename,
}

/// 人工纠正只改变 World 聚合与展示，不改写 Provider 原始 Signal。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorldCorrelationCorrection {
    pub id: WorldCorrelationCorrectionId,
    pub kind: WorldCorrelationCorrectionKind,
    pub correlation_kind: CorrelationKind,
    #[serde(default)]
    pub source_keys: Vec<String>,
    pub target_key: Option<String>,
    pub label: Option<String>,
    pub item_id: Option<WorldItemId>,
    pub corrected_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorldItemResolution {
    pub item_id: WorldItemId,
    pub kind: WorldItemResolutionKind,
    pub note: Option<String>,
    pub resolved_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PersonalBrief {
    pub generated_at: DateTime<Utc>,
    pub open_commitments: Vec<WorldItemRecord>,
    pub waiting_external: Vec<WorldItemRecord>,
    pub overdue: Vec<WorldItemRecord>,
    pub action_items_due_soon: Vec<WorldItemRecord>,
    pub risks: Vec<WorldItemRecord>,
    pub upcoming_calendar: Vec<WorldItemRecord>,
}
