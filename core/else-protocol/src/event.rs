//! 事件：一切状态是事件的投影。
//!
//! 事件只追加、不修改。任务状态、World Model、UI 都从事件重放而来；
//! 系统重启、Agent 更换、计划重生成，都不丢失"为什么发展到现在"。

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::artifact::{Artifact, Checkpoint};
use crate::attention::{AttentionRequest, AttentionWindow, Decision, Ruling};
use crate::capability::{
    AgentBinding, AgentBindingEvent, AgentBindingState, CapabilityRegistration,
    McpServerRegistration,
};
use crate::connector::{
    ConnectorAction, ConnectorActionStatus, ConnectorCredentialMetadata, ConnectorRegistration,
    ConnectorSyncStatus, ExternalFact, IntentAcceptedBy, IntentCandidate, IntentCandidateState,
    SignalInterpretation, SyncCursor, WorldSignal,
};
use crate::effect::{EffectConclusion, ToolCall, ToolOutcome};
use crate::egress::EgressDecision;
use crate::failure::ScopeFailure;
use crate::ids::*;
use crate::intent::{IntentSpec, IntentState};
use crate::plan::{Plan, PlanImpact};
use crate::resource::{ResourceBudget, ResourceLease, SandboxUsage};
use crate::world::{WorldCorrelationCorrection, WorldItemResolution};
use crate::PROTOCOL_VERSION;

/// 事件的来源。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Actor {
    User,
    System,
    Agent,
    Tool,
    External,
}

/// Scope 生命周期。状态机不能因为用户点了停止就直接变成 Cancelled：
/// 只有确认不会再产生新副作用、资源已释放、在途操作已有结论，才能进入终态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScopeState {
    Running,
    AbortRequested,
    Cancelling,
    /// 已围栏：旧 Generation 的写入与外部动作被 Gateway 拒绝。
    Fenced,
    Compensating,
    Paused,
    Completed,
    /// 上游失败或用户修订使该分支不再需要执行；保留历史，不伪装成成功。
    Superseded,
    Cancelled,
    Failed,
}

/// 事件信封。`seq`（事件水位）由 Ledger 在追加时分配，
/// 见 else-ledger 的 `StoredEvent`。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EventEnvelope {
    pub event_id: String,
    pub protocol_version: String,
    pub intent_id: IntentId,
    pub scope_id: ScopeId,
    pub run_id: Option<RunId>,
    /// Execution Generation：哪些执行与写入仍然有效。
    /// Abort 后递增，旧 Generation 的工具调用一律被 Gateway 拒绝。
    pub generation: u64,
    /// Intent Revision：目标语义版本。与 Generation 永不混用。
    pub intent_revision: u64,
    /// 本事件基于哪个 Context Snapshot。
    pub snapshot_id: Option<SnapshotId>,
    pub occurred_at: DateTime<Utc>,
    pub actor: Actor,
    pub payload: EventPayload,
}

impl EventEnvelope {
    /// 构造事件：ID、时间、协议版本自动填充；
    /// generation 与 intent_revision 必须显式给出。
    pub fn new(
        intent_id: IntentId,
        scope_id: ScopeId,
        generation: u64,
        intent_revision: u64,
        actor: Actor,
        payload: EventPayload,
    ) -> Self {
        Self {
            event_id: Uuid::new_v4().to_string(),
            protocol_version: PROTOCOL_VERSION.to_string(),
            intent_id,
            scope_id,
            run_id: None,
            generation,
            intent_revision,
            snapshot_id: None,
            occurred_at: Utc::now(),
            actor,
            payload,
        }
    }

    /// 标记本事件基于哪个 Context Snapshot。
    pub fn with_snapshot(mut self, snapshot_id: SnapshotId) -> Self {
        self.snapshot_id = Some(snapshot_id);
        self
    }
}

/// 事件负载。`type` 为 snake_case 事件名，`data` 为负载。
/// schema 演进规则：只加新变体或可选字段，不删不改已有语义。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum EventPayload {
    IntentCreated {
        spec: IntentSpec,
    },
    /// 只有用户或其明确授权的决策能产生此事件。
    IntentRevised {
        spec: IntentSpec,
        reason: String,
    },
    IntentStateChanged {
        state: IntentState,
    },

    /// Plan 是易失的：计划失败不等于 Intent 失败。
    PlanProposed {
        plan_id: String,
        summary: String,
    },
    PlanReplaced {
        plan_id: String,
        reason: String,
    },
    /// 结构化计划版本。旧 `PlanProposed` 保留用于重放历史事件。
    StructuredPlanProposed {
        plan: Plan,
    },
    /// 结构化重规划版本；前序 Plan 在投影中标记为 Superseded。
    StructuredPlanReplaced {
        plan: Plan,
        reason: String,
    },
    /// 外部事实对当前计划步骤与 Scope 的确定性影响分析。
    PlanImpactAnalyzed {
        impact: PlanImpact,
    },
    CapabilityRegistered {
        capability: CapabilityRegistration,
    },
    AgentBindingCreated {
        binding: AgentBinding,
    },
    AgentBindingRevoked {
        binding_id: AgentBindingId,
        reason: String,
    },
    AgentBindingStateChanged {
        binding_id: AgentBindingId,
        state: AgentBindingState,
    },
    AgentBindingEventReceived {
        binding_id: AgentBindingId,
        event: AgentBindingEvent,
    },
    McpServerRegistered {
        server: McpServerRegistration,
    },
    McpToolsChanged {
        server_id: McpServerId,
    },
    McpToolsRefreshed {
        server_id: McpServerId,
        capability_ids: Vec<CapabilityId>,
    },
    CapabilityRevoked {
        capability_id: CapabilityId,
        reason: String,
    },
    ConnectorRegistered {
        connector: ConnectorRegistration,
    },
    ConnectorCredentialUpdated {
        connector_id: ConnectorId,
        credential: ConnectorCredentialMetadata,
    },
    ConnectorSyncStatusChanged {
        status: ConnectorSyncStatus,
    },
    ConnectorActionProposed {
        action: ConnectorAction,
    },
    ConnectorActionStateChanged {
        action_id: ConnectorActionId,
        status: ConnectorActionStatus,
        attention_id: Option<AttentionId>,
        tool_call_id: Option<ToolCallId>,
        outcome: Option<ToolOutcome>,
        error: Option<String>,
    },
    ExternalFactIngested {
        fact: ExternalFact,
    },
    WorldSignalIngested {
        signal: WorldSignal,
    },
    WorldSignalStatusChanged {
        signal_id: WorldSignalId,
        status: crate::connector::SignalStatus,
    },
    SignalInterpreted {
        interpretation: SignalInterpretation,
    },
    IntentCandidateProposed {
        candidate: IntentCandidate,
    },
    IntentCandidateStateChanged {
        candidate_id: IntentCandidateId,
        state: IntentCandidateState,
        intent_id: Option<IntentId>,
        #[serde(default)]
        accepted_by: Option<IntentAcceptedBy>,
    },
    ConnectorSyncCheckpointed {
        cursor: SyncCursor,
    },
    ConnectorCursorReset {
        connector_id: ConnectorId,
        stream_id: String,
    },
    WorldItemResolved {
        resolution: WorldItemResolution,
    },
    WorldCorrelationCorrected {
        correction: WorldCorrelationCorrection,
    },

    RunStarted {
        run_id: RunId,
        harness: String,
        /// Harness 能力声明（MVP-2 起随启动落账）；缺省兼容旧事件。
        #[serde(default)]
        capabilities: Option<crate::harness::HarnessCapabilities>,
    },
    RunFinished {
        run_id: RunId,
        success: bool,
    },
    ScopeFailureRecorded {
        failure: ScopeFailure,
    },
    ScopeRetryApproved,

    /// Harness 桥接的归一化输出（截断后的文本块）。
    HarnessOutput {
        text: String,
    },

    /// 模型调用也是资源消耗：Token、费用入账（Accounting）。
    ModelCalled {
        model: String,
        purpose: String,
    },
    ModelCompleted {
        model: String,
        usage: crate::resource::ModelUsage,
    },

    /// 验收通过：Verifier 按原始 DoneWhen（而非最新 Plan）核对证据。
    OutcomeVerified {
        evidence: Vec<crate::artifact::OutcomeEvidence>,
    },

    ToolCalled {
        call: ToolCall,
    },
    ToolCompleted {
        call_id: ToolCallId,
        outcome: ToolOutcome,
    },
    /// 不可接管 Harness 自行执行的工具。该事件表示“已观察”，
    /// 不经过 Gateway，也不得被表面解释为“已授权”。
    OpaqueToolObserved {
        call: ToolCall,
        outcome: ToolOutcome,
        backend: String,
    },
    EgressIntercepted {
        decision: EgressDecision,
    },
    /// 在途副作用有了最终结论（已应用/已补偿/未知）。
    SideEffectConcluded {
        call_id: ToolCallId,
        conclusion: EffectConclusion,
        note: String,
    },

    ExternalEventReceived {
        source: String,
        summary: String,
    },
    ExternalWaitStarted {
        reason: String,
    },
    TimerScheduled {
        timer_id: TimerId,
        fire_at: DateTime<Utc>,
        reason: String,
    },
    TimerFired {
        timer_id: TimerId,
    },
    TimerCancelled {
        timer_id: TimerId,
        reason: String,
    },

    AttentionRequested {
        request: AttentionRequest,
    },
    AttentionRuled {
        request_id: AttentionId,
        ruling: Ruling,
    },
    /// 持久注意力窗口：启动时登记成员和截止时间。
    AttentionWindowOpened {
        window: AttentionWindow,
    },
    /// 新请求进入仍然打开的 Batch 窗口。
    AttentionWindowMemberAdded {
        window_id: AttentionWindowId,
        request_id: AttentionId,
    },
    /// 窗口截止或请求失效后关闭；关闭是形成一次聚合通知的幂等边界。
    AttentionWindowClosed {
        window_id: AttentionWindowId,
        closed_at: DateTime<Utc>,
        delivered_at: Option<DateTime<Utc>>,
    },
    DecisionResolved {
        decision: Decision,
    },

    /// 分支创建：由检查点派生独立执行 Scope。
    ScopeSpawned {
        child: ScopeId,
    },
    /// Subagent 委托：父 Scope 由事件信封标识，子权限契约随事件持久化。
    SubagentSpawned {
        child: ScopeId,
        spec: IntentSpec,
        /// 显式执行依赖；父 Scope 只定义权限与生命周期，不自动等于依赖。
        #[serde(default)]
        depends_on: Vec<ScopeId>,
    },
    /// 自动替换终态失败 Scope；替换链最多一层，避免无限重试。
    ScopeReplacementSpawned {
        child: ScopeId,
        replaces: ScopeId,
        spec: IntentSpec,
        depends_on: Vec<ScopeId>,
    },
    /// 把依赖某个失败 Scope 的下游显式接到替换 Scope。
    ScopeDependencyReplaced {
        previous: ScopeId,
        replacement: ScopeId,
        reason: String,
    },
    ScopeAbortRequested {
        reason: String,
        /// true = 只取消该 Scope 子树（如 Intent Revision 联动），
        /// 不关闭整个 Intent；缺省（None）为 Intent 级取消。
        scope_only: Option<bool>,
    },
    ScopeStateChanged {
        state: ScopeState,
    },

    CheckpointCreated {
        checkpoint: Checkpoint,
    },
    ArtifactProduced {
        artifact: Artifact,
    },

    ResourceBudgetSet {
        budget: ResourceBudget,
    },
    LeaseGranted {
        lease: ResourceLease,
    },
    LeaseRevoked {
        lease_id: LeaseId,
    },
    SandboxStarted {
        lease_id: LeaseId,
        backend: String,
        started_at: DateTime<Utc>,
    },
    SandboxFinished {
        usage: SandboxUsage,
    },

    /// 记忆候选：尚未进入权威投影，需规则校验或用户确认。
    MemoryCandidateProposed {
        record: crate::memory::MemoryRecord,
    },
    /// 记忆晋升：确认进入其作用域与分层。
    MemoryPromoted {
        memory: crate::memory::MemoryRecord,
    },
    MemoryRevoked {
        memory_id: MemoryId,
        reason: String,
    },
    /// Context Compiler 产出的最小只读快照，记录来源与事件水位。
    ContextCompiled {
        snapshot: crate::memory::ContextSnapshot,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::effect::SideEffectKind;
    use crate::intent::{AttentionPolicy, Authority};

    fn spec() -> IntentSpec {
        IntentSpec {
            id: IntentId::new(),
            revision: 1,
            outcome: "今天达到可对外发布".into(),
            done_when: vec![],
            constraints: vec!["不牺牲现有稳定性".into()],
            authority: Authority {
                max_autonomous_effect: SideEffectKind::Reversible,
                forbidden: vec!["不擅自对外发送".into()],
            },
            attention_policy: AttentionPolicy {
                allow_interrupt: true,
                batch_interval_minutes: 30,
            },
            opaque_harness_policy: crate::intent::OpaqueHarnessPolicy::default(),
            deadline: None,
            assumptions: vec![],
            project: None,
            origin: None,
            connector_policy: Default::default(),
        }
    }

    #[test]
    fn event_json_roundtrip() {
        let event = EventEnvelope::new(
            IntentId::new(),
            ScopeId::new(),
            1,
            1,
            Actor::User,
            EventPayload::IntentCreated { spec: spec() },
        );
        let json = serde_json::to_string(&event).unwrap();
        let back: EventEnvelope = serde_json::from_str(&json).unwrap();
        assert_eq!(event, back);
        assert_eq!(back.protocol_version, PROTOCOL_VERSION);
    }

    #[test]
    fn legacy_payloads_decode_without_new_lineage_fields() {
        let legacy_fact = serde_json::json!({
            "type": "external_fact_ingested",
            "data": {
                "fact": {
                    "connector_id": "legacy-connector",
                    "fact_id": "legacy-fact",
                    "source": "calendar",
                    "summary": "legacy",
                    "occurred_at": "2026-10-31T00:00:00Z",
                    "valid_until": null
                }
            }
        });
        let payload: EventPayload = serde_json::from_value(legacy_fact).unwrap();
        match payload {
            EventPayload::ExternalFactIngested { fact } => {
                assert!(fact.entity_id.is_empty());
                assert!(fact.source_version.is_empty());
            }
            other => panic!("unexpected payload: {other:?}"),
        }

        let mut legacy_intent = serde_json::to_value(spec()).unwrap();
        legacy_intent.as_object_mut().unwrap().remove("origin");
        legacy_intent
            .as_object_mut()
            .unwrap()
            .remove("connector_policy");
        let decoded: IntentSpec = serde_json::from_value(legacy_intent).unwrap();
        assert!(decoded.origin.is_none());
        assert!(!decoded.connector_policy.auto_accept_low_risk);

        let legacy_candidate_state = serde_json::json!({
            "type": "intent_candidate_state_changed",
            "data": {
                "candidate_id": "candidate-legacy",
                "state": "accepted",
                "intent_id": null
            }
        });
        let payload: EventPayload = serde_json::from_value(legacy_candidate_state).unwrap();
        match payload {
            EventPayload::IntentCandidateStateChanged { accepted_by, .. } => {
                assert!(accepted_by.is_none());
            }
            other => panic!("unexpected payload: {other:?}"),
        }

        let legacy_connector = serde_json::json!({
            "type": "connector_registered",
            "data": {
                "connector": {
                    "id": "connector-legacy",
                    "name": "legacy",
                    "kind": "calendar_read_only",
                    "source": "/tmp/calendar.ics",
                    "enabled": true
                }
            }
        });
        let payload: EventPayload = serde_json::from_value(legacy_connector).unwrap();
        match payload {
            EventPayload::ConnectorRegistered { connector } => {
                assert_eq!(connector.manifest, Default::default());
                assert!(connector.webhook_session.is_none());
            }
            other => panic!("unexpected payload: {other:?}"),
        }

        let legacy_signal = serde_json::json!({
            "type": "world_signal_ingested",
            "data": {
                "signal": {
                    "id": "signal-legacy",
                    "connector_id": "connector-legacy",
                    "entity_ref": {
                        "provider": "linear",
                        "tenant": null,
                        "account": null,
                        "entity_kind": "linear_issue",
                        "external_id": "issue-legacy",
                        "canonical_url": null,
                        "source_version": "1"
                    },
                    "signal_type": "waiting_external",
                    "actor": null,
                    "occurred_at": "2026-10-31T00:00:00Z",
                    "valid_from": null,
                    "valid_until": null,
                    "summary": "legacy waiting",
                    "structured_facts": {},
                    "evidence_ref": null,
                    "dedupe_key": "legacy-signal",
                    "sensitivity": "internal",
                    "source_trust": "provider_read_only",
                    "status": "active"
                }
            }
        });
        let payload: EventPayload = serde_json::from_value(legacy_signal).unwrap();
        match payload {
            EventPayload::WorldSignalIngested { signal } => {
                assert_eq!(
                    signal.legacy_signal_type,
                    Some(crate::connector::SignalType::WaitingExternal)
                );
                assert_eq!(signal.object_kind, crate::connector::ObjectKind::Other);
            }
            other => panic!("unexpected payload: {other:?}"),
        }
    }

    #[test]
    fn side_effect_severity_is_ordered() {
        assert!(SideEffectKind::ReadOnly < SideEffectKind::Reversible);
        assert!(SideEffectKind::Irreversible < SideEffectKind::Unknown);
    }
}
