//! 外部世界连接器：首批默认只读，事实必须带来源与有效期。

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::effect::{SideEffectKind, ToolOutcome};
use crate::ids::{ConnectorActionId, ConnectorId, IntentCandidateId, IntentId, WorldSignalId};
use crate::intent::{AttentionPolicy, Authority, Criterion};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnectorAuthMode {
    #[default]
    None,
    ApiToken,
    OAuth,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnectorSyncMode {
    Pull,
    Webhook,
    Hybrid,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnectorHealth {
    #[default]
    Healthy,
    Degraded,
    Error,
}

/// 凭据只存在于 Keychain 或 Runtime 私有凭据区。Ledger 仅记录本结构的
/// 非敏感元数据；字段名保持独立，避免把凭据误嵌进 Connector 事件。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConnectorCredentialMetadata {
    pub reference: String,
    pub auth_mode: ConnectorAuthMode,
    #[serde(default)]
    pub scopes: Vec<String>,
    pub expires_at: Option<DateTime<Utc>>,
    pub configured_at: DateTime<Utc>,
    pub revoked_at: Option<DateTime<Utc>>,
}

/// OAuth refresh 所需配置会与 access/refresh token 一起写入私有凭据区。
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OAuthRefreshConfig {
    pub token_url: String,
    pub client_id: String,
    pub client_secret: Option<String>,
}

impl std::fmt::Debug for OAuthRefreshConfig {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("OAuthRefreshConfig")
            .field("token_url", &self.token_url)
            .field("client_id", &self.client_id)
            .field(
                "client_secret",
                &self.client_secret.as_ref().map(|_| "<redacted>"),
            )
            .finish()
    }
}

/// Adapter 在执行 pull 时临时取得的凭据。该结构不得写入 Event Ledger。
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConnectorCredential {
    pub access_token: String,
    pub refresh_token: Option<String>,
    #[serde(default = "default_token_type")]
    pub token_type: String,
    pub scopes: Vec<String>,
    pub expires_at: Option<DateTime<Utc>>,
    pub refresh: Option<OAuthRefreshConfig>,
}

impl std::fmt::Debug for ConnectorCredential {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ConnectorCredential")
            .field("access_token", &"<redacted>")
            .field(
                "refresh_token",
                &self.refresh_token.as_ref().map(|_| "<redacted>"),
            )
            .field("token_type", &self.token_type)
            .field("scopes", &self.scopes)
            .field("expires_at", &self.expires_at)
            .field("refresh", &self.refresh)
            .finish()
    }
}

impl ConnectorCredential {
    pub fn validate_scopes(&self, required: &[String]) -> std::result::Result<(), String> {
        let missing = required
            .iter()
            .filter(|scope| !self.scopes.contains(scope))
            .cloned()
            .collect::<Vec<_>>();
        if missing.is_empty() {
            Ok(())
        } else {
            Err(format!(
                "credential is missing required scopes: {}",
                missing.join(", ")
            ))
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConnectorSyncStatus {
    pub connector_id: ConnectorId,
    pub last_attempt_at: Option<DateTime<Utc>>,
    pub last_success_at: Option<DateTime<Utc>>,
    pub consecutive_failures: u32,
    pub last_error: Option<String>,
    pub next_sync_at: DateTime<Utc>,
}

impl ConnectorSyncStatus {
    pub fn health(&self) -> ConnectorHealth {
        match self.consecutive_failures {
            0 => ConnectorHealth::Healthy,
            1..=2 => ConnectorHealth::Degraded,
            _ => ConnectorHealth::Error,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConnectorHealthReport {
    pub connector_id: ConnectorId,
    pub provider: String,
    pub health: ConnectorHealth,
    pub credential_configured: bool,
    pub credential_scopes: Vec<String>,
    pub credential_expires_at: Option<DateTime<Utc>>,
    pub last_attempt_at: Option<DateTime<Utc>>,
    pub last_success_at: Option<DateTime<Utc>>,
    pub consecutive_failures: u32,
    pub last_error: Option<String>,
    pub next_sync_at: DateTime<Utc>,
    #[serde(default)]
    pub backfill_in_progress: bool,
    #[serde(default)]
    pub backfill_started_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub backfill_completed_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub pages_synced: u64,
    pub checked_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnectorActionStatus {
    Proposed,
    NeedsApproval,
    Executing,
    Applied,
    Compensating,
    CompensationExecuting,
    Compensated,
    CompensationFailed,
    Failed,
    Unknown,
    Cancelled,
}

/// Provider 写回的持久动作。真实请求只能通过 Tool Gateway 执行；
/// 记录中保存目标与参数，但不保存凭据。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConnectorAction {
    pub id: ConnectorActionId,
    pub connector_id: ConnectorId,
    pub capability: String,
    pub target: String,
    pub input: serde_json::Value,
    pub effect: SideEffectKind,
    pub idempotency_key: String,
    pub status: ConnectorActionStatus,
    pub proposed_at: DateTime<Utc>,
    pub attention_id: Option<crate::ids::AttentionId>,
    pub tool_call_id: Option<crate::ids::ToolCallId>,
    pub outcome: Option<ToolOutcome>,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConnectorManifest {
    #[serde(default)]
    pub adapter: String,
    pub provider: String,
    pub auth_mode: ConnectorAuthMode,
    #[serde(default)]
    pub scopes: Vec<String>,
    #[serde(default)]
    pub read_capabilities: Vec<String>,
    #[serde(default)]
    pub write_capabilities: Vec<String>,
    #[serde(default)]
    pub sync_modes: Vec<ConnectorSyncMode>,
    pub data_sensitivity: String,
    pub retention_policy: String,
    pub health: ConnectorHealth,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConnectorRegistration {
    pub id: ConnectorId,
    pub name: String,
    /// Adapter Registry 中的稳定标识；`kind` 是旧事件兼容别名。
    #[serde(default, alias = "kind")]
    pub adapter: String,
    /// 本地文件路径或 HTTPS URL。
    pub source: String,
    pub enabled: bool,
    #[serde(default)]
    pub manifest: ConnectorManifest,
    /// Provider Webhook 使用的独立 Session Token，不暴露 daemon 管理 Token。
    #[serde(default)]
    pub webhook_session: Option<String>,
    /// 建议同步间隔；调度状态由 ConnectorSyncStatus 事件持久化。
    #[serde(default = "default_sync_interval_seconds")]
    pub sync_interval_seconds: u64,
    #[serde(default)]
    pub registered_at: Option<DateTime<Utc>>,
    /// 秘密本体不在 Ledger；这里仅保存引用、Scope 与过期时间。
    #[serde(default)]
    pub credential: Option<ConnectorCredentialMetadata>,
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
    #[serde(default)]
    pub entity_kind: String,
    pub canonical_url: Option<String>,
    #[serde(default)]
    pub signal_type: SignalType,
    pub actor: Option<String>,
    #[serde(default)]
    pub structured_facts: serde_json::Value,
    pub evidence_ref: Option<String>,
    #[serde(default = "default_sensitivity")]
    pub sensitivity: String,
    #[serde(default = "default_source_trust")]
    pub source_trust: String,
    #[serde(default)]
    pub status: SignalStatus,
}

/// Provider 原始事件的规范化观察。Adapter 只能产出这一层，不解释业务语义。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConnectorObservation {
    pub connector_id: ConnectorId,
    pub entity_id: String,
    pub source_version: String,
    pub dedupe_key: String,
    pub source: String,
    pub summary: String,
    pub occurred_at: DateTime<Utc>,
    pub valid_until: Option<DateTime<Utc>>,
    pub entity_kind: String,
    pub canonical_url: Option<String>,
    pub object_kind: ObjectKind,
    pub change: ObservationChange,
    pub actor: Option<String>,
    #[serde(default)]
    pub attributes: serde_json::Value,
    pub evidence_ref: Option<String>,
    pub sensitivity: String,
    pub source_trust: String,
    #[serde(default)]
    pub correlations: Vec<CorrelationHint>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObjectKind {
    CalendarEvent,
    WorkItem,
    Milestone,
    Message,
    Document,
    #[default]
    Other,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObservationChange {
    #[default]
    Upsert,
    Delete,
    Supersede,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CorrelationKind {
    Project,
    Contact,
    Customer,
    Thread,
    Milestone,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CorrelationHint {
    pub kind: CorrelationKind,
    pub key: String,
    pub label: String,
}

/// 仅用于读取 Phase 1/2 历史事件；新 Adapter 不再产出业务语义类型。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SignalType {
    #[default]
    CalendarEvent,
    Message,
    Issue,
    Page,
    Commitment,
    ActionItem,
    WaitingExternal,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SignalStatus {
    #[default]
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
    #[serde(default)]
    pub object_kind: ObjectKind,
    #[serde(default)]
    pub change: ObservationChange,
    /// 历史兼容字段，新 Signal 不再序列化业务语义。
    #[serde(
        default,
        rename = "signal_type",
        skip_serializing_if = "Option::is_none"
    )]
    pub legacy_signal_type: Option<SignalType>,
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
    #[serde(default)]
    pub correlations: Vec<CorrelationHint>,
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
    #[serde(default)]
    pub accepted_by: Option<IntentAcceptedBy>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SignalMeaning {
    ActionItem,
    Commitment,
    WaitingExternal,
    Risk,
    Context,
}

/// 对中性 WorldSignal 的确定性业务解释。Resolver 只消费这一层。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SignalInterpretation {
    pub signal_id: WorldSignalId,
    pub meaning: SignalMeaning,
    pub confidence: u8,
    #[serde(default)]
    pub basis: Vec<String>,
    pub deadline: Option<DateTime<Utc>>,
    pub ruleset_version: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncCursor {
    pub connector_id: ConnectorId,
    pub stream_id: String,
    pub opaque_cursor: String,
    pub overlap_until: Option<DateTime<Utc>>,
    pub checkpointed_at: DateTime<Utc>,
    /// Adapter 的不透明下一页 continuation；存在时表示 Backfill/分页尚未完成。
    #[serde(default)]
    pub continuation: Option<String>,
    #[serde(default)]
    pub pages_synced: u64,
    #[serde(default)]
    pub backfill_started_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub backfill_completed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorldSignalRecord {
    pub seq: u64,
    pub intent_id: IntentId,
    pub signal: WorldSignal,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IntentCandidateRecord {
    pub seq: u64,
    pub intent_id: IntentId,
    pub candidate: IntentCandidate,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConnectorPullPage {
    pub stream_id: String,
    pub opaque_cursor: String,
    pub overlap_until: Option<DateTime<Utc>>,
    /// 有值表示 Runtime 应立即以该 continuation 拉取下一页。
    #[serde(default)]
    pub continuation: Option<String>,
    pub observations: Vec<ConnectorObservation>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConnectorPolicy {
    pub auto_accept_low_risk: bool,
    pub allowed_origins: Vec<IntentOriginKind>,
    pub min_confidence: u8,
    pub max_autonomous_effect: SideEffectKind,
}

impl Default for ConnectorPolicy {
    fn default() -> Self {
        Self {
            auto_accept_low_risk: false,
            allowed_origins: vec![IntentOriginKind::Commitment, IntentOriginKind::InferredNeed],
            min_confidence: 80,
            max_autonomous_effect: SideEffectKind::ReadOnly,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConnectorSyncResult {
    pub observed: u64,
    pub inserted: u64,
    pub skipped: u64,
    pub candidates_proposed: u64,
    pub auto_accepted: u64,
    #[serde(default)]
    pub cursor: Option<String>,
    #[serde(default)]
    pub pages_synced: u64,
    #[serde(default)]
    pub has_more: bool,
    #[serde(default)]
    pub backfill_complete: bool,
    #[serde(default)]
    pub hydrated: bool,
}

fn default_sensitivity() -> String {
    "internal".into()
}

fn default_source_trust() -> String {
    "provider_read_only".into()
}

fn default_token_type() -> String {
    "bearer".into()
}

fn default_sync_interval_seconds() -> u64 {
    300
}
