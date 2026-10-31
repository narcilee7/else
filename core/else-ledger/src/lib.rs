//! else Event Ledger：SQLite 上的追加式事件存储与确定性重放。
//!
//! 事件只追加、不修改；`seq` 是全局单调的事件水位。
//! 所有状态（Intent State、Decision Queue、UI）都是事件的投影：
//! 进程崩溃后重开数据库重放，投影必须一致——这是 M0 的永久回归测试。

mod projection;

pub use projection::{
    AttentionMetrics, AttentionRecordProjection, IntentProjection, ScopeProjection, TimerProjection,
};

use std::cell::Cell;

use else_protocol::event::EventEnvelope;
use else_protocol::ids::IntentId;
use else_protocol::PROTOCOL_VERSION;
use rusqlite::{params, Connection};
use thiserror::Error;

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS events (
    seq              INTEGER PRIMARY KEY AUTOINCREMENT,
    event_id         TEXT NOT NULL UNIQUE,
    protocol_version TEXT NOT NULL,
    intent_id        TEXT NOT NULL,
    scope_id         TEXT NOT NULL,
    generation       INTEGER NOT NULL,
    intent_revision  INTEGER NOT NULL,
    actor            TEXT NOT NULL,
    event_type       TEXT NOT NULL,
    occurred_at      TEXT NOT NULL,
    payload          TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_events_intent ON events(intent_id, seq);
CREATE INDEX IF NOT EXISTS idx_events_scope  ON events(scope_id, seq);
"#;

#[derive(Debug, Error)]
pub enum LedgerError {
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("event codec: {0}")]
    Codec(#[from] serde_json::Error),
    #[error("unsupported protocol version: {found}, expected {expected}")]
    UnsupportedVersion { found: String, expected: String },
    /// 故障注入：事件已持久化，随后模拟进程崩溃。
    #[error("crash injected after durable event append")]
    CrashInjected,
}

pub type Result<T> = std::result::Result<T, LedgerError>;

/// 已落盘的事件：`seq` 即事件水位，检查点的 `event_cursor` 引用它。
#[derive(Debug, Clone, PartialEq)]
pub struct StoredEvent {
    pub seq: u64,
    pub envelope: EventEnvelope,
}

/// 投影：从事件流重建的状态。投影必须可丢弃、可重建。
pub trait Projection {
    fn apply(&mut self, event: &StoredEvent);
}

pub struct Ledger {
    conn: Connection,
    crash_after_appends: Cell<Option<u64>>,
}

impl Ledger {
    /// 打开（或创建）事件库。WAL 模式保证崩溃后的持久性。
    pub fn open(path: impl AsRef<std::path::Path>) -> Result<Self> {
        let conn = Connection::open(path)?;
        Self::init(conn)
    }

    pub fn open_in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> Result<Self> {
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        // 单写者多连接下（如 CLI 并行操作同一 Runtime 目录）等待而非立即失败。
        conn.pragma_update(None, "busy_timeout", 5_000)?;
        conn.execute_batch(SCHEMA)?;
        Ok(Self {
            conn,
            crash_after_appends: Cell::new(None),
        })
    }

    /// 测试故障注入：再持久化 N 条事件后返回 CrashInjected。
    /// 调用方丢弃 Ledger 并重开，即模拟进程在事件边界被 kill。
    #[doc(hidden)]
    pub fn crash_after_appends(&self, appends: u64) {
        self.crash_after_appends.set(Some(appends));
    }

    /// 追加事件并返回分配了 `seq` 的 StoredEvent。
    pub fn append(&self, envelope: &EventEnvelope) -> Result<StoredEvent> {
        let payload = serde_json::to_string(envelope)?;
        let event_type = event_type_name(&envelope.payload);
        let actor = format!("{:?}", envelope.actor).to_lowercase();
        self.conn.execute(
            "INSERT INTO events
             (event_id, protocol_version, intent_id, scope_id, generation,
              intent_revision, actor, event_type, occurred_at, payload)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                envelope.event_id,
                envelope.protocol_version,
                envelope.intent_id.to_string(),
                envelope.scope_id.to_string(),
                envelope.generation as i64,
                envelope.intent_revision as i64,
                actor,
                event_type,
                envelope.occurred_at.to_rfc3339(),
                payload,
            ],
        )?;
        if let Some(remaining) = self.crash_after_appends.get() {
            if remaining <= 1 {
                self.crash_after_appends.set(None);
                return Err(LedgerError::CrashInjected);
            }
            self.crash_after_appends.set(Some(remaining - 1));
        }
        let seq = self.conn.last_insert_rowid() as u64;
        Ok(StoredEvent {
            seq,
            envelope: envelope.clone(),
        })
    }

    /// 当前事件水位（最大 seq），用于创建检查点。
    pub fn cursor(&self) -> Result<u64> {
        let seq: i64 =
            self.conn
                .query_row("SELECT COALESCE(MAX(seq), 0) FROM events", [], |row| {
                    row.get(0)
                })?;
        Ok(seq as u64)
    }

    /// 按 seq 升序读取全部事件。
    pub fn read_all(&self) -> Result<Vec<StoredEvent>> {
        self.read_where("1 = 1", &[])
    }

    /// 按 seq 升序读取某个 Intent 的事件（局部重放）。
    pub fn read_intent(&self, intent_id: &IntentId) -> Result<Vec<StoredEvent>> {
        self.read_where("intent_id = ?1", &[&intent_id.to_string()])
    }

    /// 从检查点水位之后继续读取（增量重放）。
    pub fn read_after(&self, cursor: u64) -> Result<Vec<StoredEvent>> {
        self.read_where("seq > ?1", &[&(cursor as i64)])
    }

    fn read_where(
        &self,
        condition: &str,
        params: &[&dyn rusqlite::ToSql],
    ) -> Result<Vec<StoredEvent>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT seq, payload FROM events WHERE {condition} ORDER BY seq"
        ))?;
        let rows = stmt.query_map(params, |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        })?;
        let mut events = Vec::new();
        for row in rows {
            let (seq, payload) = row?;
            let envelope: EventEnvelope = serde_json::from_str(&payload)?;
            check_version(&envelope.protocol_version)?;
            events.push(StoredEvent {
                seq: seq as u64,
                envelope,
            });
        }
        Ok(events)
    }

    /// 全量重放到投影。
    pub fn replay<P: Projection>(&self, projection: &mut P) -> Result<()> {
        for event in self.read_all()? {
            projection.apply(&event);
        }
        Ok(())
    }
}

/// 协议版本兼容检查。0.x 阶段要求严格一致；
/// 1.0 后按 major 兼容并提供迁移。
fn check_version(found: &str) -> Result<()> {
    if found != PROTOCOL_VERSION {
        return Err(LedgerError::UnsupportedVersion {
            found: found.to_string(),
            expected: PROTOCOL_VERSION.to_string(),
        });
    }
    Ok(())
}

fn event_type_name(payload: &else_protocol::event::EventPayload) -> &'static str {
    use else_protocol::event::EventPayload as P;
    match payload {
        P::IntentCreated { .. } => "intent_created",
        P::IntentRevised { .. } => "intent_revised",
        P::IntentStateChanged { .. } => "intent_state_changed",
        P::PlanProposed { .. } => "plan_proposed",
        P::PlanReplaced { .. } => "plan_replaced",
        P::StructuredPlanProposed { .. } => "structured_plan_proposed",
        P::StructuredPlanReplaced { .. } => "structured_plan_replaced",
        P::PlanImpactAnalyzed { .. } => "plan_impact_analyzed",
        P::CapabilityRegistered { .. } => "capability_registered",
        P::AgentBindingCreated { .. } => "agent_binding_created",
        P::AgentBindingRevoked { .. } => "agent_binding_revoked",
        P::AgentBindingStateChanged { .. } => "agent_binding_state_changed",
        P::AgentBindingEventReceived { .. } => "agent_binding_event_received",
        P::McpServerRegistered { .. } => "mcp_server_registered",
        P::McpToolsChanged { .. } => "mcp_tools_changed",
        P::McpToolsRefreshed { .. } => "mcp_tools_refreshed",
        P::CapabilityRevoked { .. } => "capability_revoked",
        P::ConnectorRegistered { .. } => "connector_registered",
        P::ConnectorCredentialUpdated { .. } => "connector_credential_updated",
        P::ConnectorSyncStatusChanged { .. } => "connector_sync_status_changed",
        P::ConnectorActionProposed { .. } => "connector_action_proposed",
        P::ConnectorActionStateChanged { .. } => "connector_action_state_changed",
        P::ConnectorSyncCheckpointed { .. } => "connector_sync_checkpointed",
        P::WorldItemResolved { .. } => "world_item_resolved",
        P::ExternalFactIngested { .. } => "external_fact_ingested",
        P::WorldSignalIngested { .. } => "world_signal_ingested",
        P::WorldSignalStatusChanged { .. } => "world_signal_status_changed",
        P::SignalInterpreted { .. } => "signal_interpreted",
        P::IntentCandidateProposed { .. } => "intent_candidate_proposed",
        P::IntentCandidateStateChanged { .. } => "intent_candidate_state_changed",
        P::RunStarted { .. } => "run_started",
        P::RunFinished { .. } => "run_finished",
        P::ScopeFailureRecorded { .. } => "scope_failure_recorded",
        P::HarnessOutput { .. } => "harness_output",
        P::ModelCalled { .. } => "model_called",
        P::ModelCompleted { .. } => "model_completed",
        P::OutcomeVerified { .. } => "outcome_verified",
        P::ToolCalled { .. } => "tool_called",
        P::ToolCompleted { .. } => "tool_completed",
        P::OpaqueToolObserved { .. } => "opaque_tool_observed",
        P::EgressIntercepted { .. } => "egress_intercepted",
        P::SideEffectConcluded { .. } => "side_effect_concluded",
        P::ExternalEventReceived { .. } => "external_event_received",
        P::ExternalWaitStarted { .. } => "external_wait_started",
        P::TimerScheduled { .. } => "timer_scheduled",
        P::TimerFired { .. } => "timer_fired",
        P::TimerCancelled { .. } => "timer_cancelled",
        P::AttentionRequested { .. } => "attention_requested",
        P::AttentionRuled { .. } => "attention_ruled",
        P::AttentionWindowOpened { .. } => "attention_window_opened",
        P::AttentionWindowMemberAdded { .. } => "attention_window_member_added",
        P::AttentionWindowClosed { .. } => "attention_window_closed",
        P::DecisionResolved { .. } => "decision_resolved",
        P::ScopeSpawned { .. } => "scope_spawned",
        P::SubagentSpawned { .. } => "subagent_spawned",
        P::ScopeReplacementSpawned { .. } => "scope_replacement_spawned",
        P::ScopeDependencyReplaced { .. } => "scope_dependency_replaced",
        P::ScopeAbortRequested { .. } => "scope_abort_requested",
        P::ScopeStateChanged { .. } => "scope_state_changed",
        P::CheckpointCreated { .. } => "checkpoint_created",
        P::ArtifactProduced { .. } => "artifact_produced",
        P::ResourceBudgetSet { .. } => "resource_budget_set",
        P::LeaseGranted { .. } => "lease_granted",
        P::LeaseRevoked { .. } => "lease_revoked",
        P::SandboxStarted { .. } => "sandbox_started",
        P::SandboxFinished { .. } => "sandbox_finished",
        P::MemoryCandidateProposed { .. } => "memory_candidate_proposed",
        P::MemoryPromoted { .. } => "memory_promoted",
        P::MemoryRevoked { .. } => "memory_revoked",
        P::ContextCompiled { .. } => "context_compiled",
    }
}
