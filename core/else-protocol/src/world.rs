//! Personal World：从 Signal Interpretation 聚合出的承诺、等待与行动状态。

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::connector::{ExternalEntityRef, ObjectKind};
use crate::ids::{ConnectorId, IntentId, WorldItemId, WorldSignalId};

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
    pub signal_ids: Vec<WorldSignalId>,
    pub connector_ids: Vec<ConnectorId>,
    pub first_observed_at: DateTime<Utc>,
    pub last_observed_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorldItemRecord {
    pub intent_id: IntentId,
    pub item: WorldItem,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorldItemResolutionKind {
    Completed,
    Dismissed,
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
