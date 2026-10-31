//! 外部世界连接器：首批默认只读，事实必须带来源与有效期。

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::ids::{ConnectorId, IntentCandidateId, IntentId, WorldSignalId};
use crate::intent::{AttentionPolicy, Authority, Criterion};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnectorKind {
    CalendarReadOnly,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConnectorRegistration {
    pub id: ConnectorId,
    pub name: String,
    pub kind: ConnectorKind,
    /// 本地文件路径或 HTTPS URL。
    pub source: String,
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExternalFact {
    pub connector_id: ConnectorId,
    /// Provider 内稳定实体身份；版本变化时保持不变。
    #[serde(default)]
    pub entity_id: String,
    /// 当前内容版本；同一实体的编辑会产生新版本。
    #[serde(default)]
    pub source_version: String,
    /// 包含 Provider、实体身份与内容版本的幂等键。
    pub fact_id: String,
    pub source: String,
    pub summary: String,
    pub occurred_at: DateTime<Utc>,
    pub valid_until: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SignalType {
    Message,
    Issue,
    Page,
    CalendarEvent,
    Commitment,
    ActionItem,
    WaitingExternal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SignalStatus {
    Active,
    Deleted,
    Superseded,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExternalEntityRef {
    pub provider: String,
    pub tenant: Option<String>,
    pub account: Option<String>,
    pub entity_kind: String,
    pub external_id: String,
    pub canonical_url: Option<String>,
    pub source_version: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorldSignal {
    pub id: WorldSignalId,
    pub connector_id: ConnectorId,
    pub entity_ref: ExternalEntityRef,
    pub signal_type: SignalType,
    pub actor: Option<String>,
    pub occurred_at: DateTime<Utc>,
    pub valid_from: Option<DateTime<Utc>>,
    pub valid_until: Option<DateTime<Utc>>,
    pub summary: String,
    #[serde(default)]
    pub structured_facts: serde_json::Value,
    pub evidence_ref: Option<String>,
    pub dedupe_key: String,
    pub sensitivity: String,
    pub source_trust: String,
    pub status: SignalStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IntentOriginKind {
    UserDirect,
    ExternalRequest,
    Commitment,
    InferredNeed,
    AgentProposal,
    SystemRecovery,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IntentAcceptedBy {
    User,
    DelegatedPolicy,
    AutonomousRule,
}

/// Intent 的完整来源血缘。它只解释来源，不扩大 Intent Authority。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IntentOrigin {
    pub kind: IntentOriginKind,
    #[serde(default)]
    pub connector_ids: Vec<ConnectorId>,
    #[serde(default)]
    pub signal_ids: Vec<WorldSignalId>,
    pub candidate_id: Option<IntentCandidateId>,
    pub accepted_by: IntentAcceptedBy,
    pub accepted_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IntentCandidateState {
    Proposed,
    Accepted,
    Rejected,
    Superseded,
    Expired,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IntentCandidate {
    pub id: IntentCandidateId,
    pub origin: IntentOriginKind,
    pub proposed_outcome: String,
    pub done_when: Vec<Criterion>,
    pub constraints: Vec<String>,
    pub authority: Authority,
    pub attention_policy: AttentionPolicy,
    pub deadline: Option<DateTime<Utc>>,
    pub related_signal_ids: Vec<WorldSignalId>,
    #[serde(default)]
    pub related_intent_id: Option<IntentId>,
    pub confidence: u8,
    pub dedupe_key: String,
    pub state: IntentCandidateState,
    pub accepted_intent_id: Option<IntentId>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConnectorSyncResult {
    pub observed: u64,
    pub inserted: u64,
    pub skipped: u64,
    pub candidates_proposed: u64,
}
