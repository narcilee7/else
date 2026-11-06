//! 常驻 Kernel supervisor：单线程持有 Runtime，命令到达或 Timer 到期时 reconcile。

use std::path::Path;
use std::sync::mpsc::{self, RecvTimeoutError, SyncSender};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use chrono::{DateTime, Utc};
use else_context::MemoryProjection;
use else_ledger::{StoredEvent, ThreadProjection};
use else_protocol::attention::{AttentionRequest, DecisionChoice};
use else_protocol::capability::{
    AgentBinding, AgentBindingEvent, AgentBindingState, AgentTransport, CapabilityRegistration,
};
use else_protocol::connector::{
    ConnectorAction, ConnectorActionPreview, ConnectorCredential, ConnectorCredentialMetadata,
    ConnectorHealthReport, ConnectorManifest, ConnectorPolicy, ConnectorRegistration,
    ConnectorSyncResult, ThreadCandidate,
};
use else_protocol::effect::SideEffectKind;
use else_protocol::ids::{
    AgentBindingId, AgentId, AttentionId, CapabilityId, CheckpointId, ConnectorActionId,
    ConnectorId, MemoryId, RunId, ScopeId, ThreadCandidateId, ThreadId, TimerId,
};
use else_protocol::memory::{MemoryRecord, MemoryScope};
use else_protocol::plan::{Plan, PlanImpact};
use else_protocol::thread::{OpaqueHarnessPolicy, ThreadSpec};

use crate::kernel::KernelDrive;
use crate::resources::ResourceReport;
use crate::{
    ApiError, ConnectorRepairReport, LeaseCleanupReport, Result, Runtime, RuntimeDataExport,
    RuntimeDiagnosticReport, RuntimeSettings, ThreadDeletionReport, ThreadRevision,
};

type Reply<T> = SyncSender<Result<T>>;

enum Command {
    CreateThread {
        outcome: String,
        project: Option<String>,
        mode: else_protocol::thread::ThreadMode,
        reply: Reply<ThreadSpec>,
    },
    Projection {
        thread_id: ThreadId,
        reply: Reply<ThreadProjection>,
    },
    ListThreads {
        reply: Reply<Vec<ThreadSpec>>,
    },
    Events {
        thread_id: ThreadId,
        reply: Reply<Vec<StoredEvent>>,
    },
    ExportData {
        reply: Reply<RuntimeDataExport>,
    },
    DeleteThread {
        thread_id: ThreadId,
        reply: Reply<ThreadDeletionReport>,
    },
    RetryFailedScopes {
        thread_id: ThreadId,
        reply: Reply<(Vec<ScopeId>, ThreadProjection)>,
    },
    GetRuntimeSettings {
        reply: Reply<RuntimeSettings>,
    },
    UpdateRuntimeSettings {
        settings: RuntimeSettings,
        reply: Reply<RuntimeSettings>,
    },
    Diagnostics {
        reply: Reply<RuntimeDiagnosticReport>,
    },
    CleanupOrphanLeases {
        reply: Reply<LeaseCleanupReport>,
    },
    Capabilities {
        thread_id: ThreadId,
        reply: Reply<Vec<CapabilityRegistration>>,
    },
    Bindings {
        thread_id: ThreadId,
        reply: Reply<Vec<AgentBinding>>,
    },
    RegisterCapability {
        thread_id: ThreadId,
        capability: CapabilityRegistration,
        reply: Reply<()>,
    },
    DiscoverMcp {
        thread_id: ThreadId,
        server_name: String,
        command: String,
        reply: Reply<Vec<CapabilityRegistration>>,
    },
    BindAgent {
        thread_id: ThreadId,
        agent_id: AgentId,
        name: String,
        scope_id: ScopeId,
        run_id: Option<RunId>,
        outbound: AgentTransport,
        capability_ids: Vec<CapabilityId>,
        ttl_seconds: i64,
        reply: Reply<AgentBinding>,
    },
    RevokeBinding {
        thread_id: ThreadId,
        binding_id: AgentBindingId,
        reason: String,
        reply: Reply<()>,
    },
    DispatchBinding {
        thread_id: ThreadId,
        binding_id: AgentBindingId,
        reply: Reply<ThreadProjection>,
    },
    RecordBindingEvent {
        thread_id: ThreadId,
        binding_id: AgentBindingId,
        session: String,
        event: AgentBindingEvent,
        reply: Reply<ThreadProjection>,
    },
    TransitionBinding {
        thread_id: ThreadId,
        binding_id: AgentBindingId,
        state: AgentBindingState,
        reply: Reply<()>,
    },
    RegisterCalendar {
        thread_id: ThreadId,
        name: String,
        source: String,
        reply: Reply<ConnectorRegistration>,
    },
    RegisterLinear {
        thread_id: ThreadId,
        name: String,
        source: String,
        reply: Reply<ConnectorRegistration>,
    },
    RegisterConnectorAdapter {
        thread_id: ThreadId,
        name: String,
        source: String,
        adapter: String,
        credential: Option<ConnectorCredential>,
        sync_interval_seconds: u64,
        reply: Reply<ConnectorRegistration>,
    },
    AttachConnectorCredential {
        thread_id: ThreadId,
        connector_id: ConnectorId,
        credential: ConnectorCredential,
        reply: Reply<ConnectorCredentialMetadata>,
    },
    RevokeConnectorCredential {
        thread_id: ThreadId,
        connector_id: ConnectorId,
        reply: Reply<ConnectorCredentialMetadata>,
    },
    ConnectorHealth {
        thread_id: ThreadId,
        connector_id: ConnectorId,
        reply: Reply<ConnectorHealthReport>,
    },
    PreviewConnectorAction {
        thread_id: ThreadId,
        connector_id: ConnectorId,
        capability: String,
        target: String,
        input: serde_json::Value,
        reply: Reply<ConnectorActionPreview>,
    },
    ProposeConnectorAction {
        thread_id: ThreadId,
        connector_id: ConnectorId,
        capability: String,
        target: String,
        input: serde_json::Value,
        idempotency_key: Option<String>,
        reply: Reply<ConnectorAction>,
    },
    ExecuteConnectorAction {
        thread_id: ThreadId,
        action_id: ConnectorActionId,
        reply: Reply<ConnectorAction>,
    },
    VerifyConnectorAction {
        thread_id: ThreadId,
        action_id: ConnectorActionId,
        reply: Reply<ConnectorAction>,
    },
    CompensateConnectorAction {
        thread_id: ThreadId,
        action_id: ConnectorActionId,
        reply: Reply<ConnectorAction>,
    },
    ConnectorActions {
        thread_id: ThreadId,
        reply: Reply<Vec<ConnectorAction>>,
    },
    ConnectorAdapters {
        reply: Reply<Vec<ConnectorManifest>>,
    },
    SyncConnector {
        thread_id: ThreadId,
        connector_id: ConnectorId,
        reply: Reply<ConnectorSyncResult>,
    },
    RepairConnector {
        thread_id: ThreadId,
        connector_id: ConnectorId,
        reply: Reply<ConnectorRepairReport>,
    },
    IngestConnectorWebhook {
        thread_id: ThreadId,
        connector_id: ConnectorId,
        session_token: String,
        payload: serde_json::Value,
        reply: Reply<ConnectorSyncResult>,
    },
    ConfigureConnectorPolicy {
        thread_id: ThreadId,
        policy: ConnectorPolicy,
        reply: Reply<ThreadSpec>,
    },
    WorldSignals {
        cursor: u64,
        provider: Option<String>,
        entity: Option<String>,
        reply: Reply<Vec<else_protocol::connector::WorldSignalRecord>>,
    },
    ThreadCandidates {
        cursor: u64,
        reply: Reply<Vec<else_protocol::connector::ThreadCandidateRecord>>,
    },
    WorldItems {
        thread_id: ThreadId,
        include_resolved: bool,
        reply: Reply<Vec<else_protocol::world::WorldItem>>,
    },
    PersonalBrief {
        reply: Reply<else_protocol::world::PersonalBrief>,
    },
    ResolveWorldItem {
        thread_id: ThreadId,
        item_id: else_protocol::ids::WorldItemId,
        kind: else_protocol::world::WorldItemResolutionKind,
        note: Option<String>,
        reply: Reply<else_protocol::world::WorldItem>,
    },
    CorrectWorldCorrelation {
        thread_id: ThreadId,
        correction: else_protocol::world::WorldCorrelationCorrection,
        reply: Reply<else_protocol::world::WorldCorrelationCorrection>,
    },
    ProjectWorlds {
        thread_id: Option<ThreadId>,
        include_resolved: bool,
        reply: Reply<Vec<else_protocol::world::ProjectWorld>>,
    },
    IdentityWorlds {
        thread_id: Option<ThreadId>,
        include_resolved: bool,
        reply: Reply<Vec<else_protocol::world::IdentityWorld>>,
    },
    ResolveSignals {
        thread_id: ThreadId,
        reply: Reply<Vec<ThreadCandidate>>,
    },
    ProposeCandidate {
        thread_id: ThreadId,
        candidate: ThreadCandidate,
        reply: Reply<ThreadCandidate>,
    },
    ResolveCandidate {
        thread_id: ThreadId,
        candidate_id: ThreadCandidateId,
        accept: bool,
        reply: Reply<Option<ThreadSpec>>,
    },
    RecordExternalEvent {
        thread_id: ThreadId,
        source: String,
        summary: String,
        reply: Reply<ThreadProjection>,
    },
    WaitForExternal {
        thread_id: ThreadId,
        reason: String,
        reply: Reply<ThreadProjection>,
    },
    ScheduleTimer {
        thread_id: ThreadId,
        fire_at: DateTime<Utc>,
        reason: String,
        reply: Reply<TimerId>,
    },
    Decide {
        thread_id: ThreadId,
        request_id: AttentionId,
        choice: DecisionChoice,
        reply: Reply<ThreadProjection>,
    },
    SubmitAttention {
        thread_id: ThreadId,
        scope_id: Option<ScopeId>,
        request: AttentionRequest,
        reply: Reply<AttentionId>,
    },
    ProposePlan {
        thread_id: ThreadId,
        plan: Plan,
        reply: Reply<Plan>,
    },
    Abort {
        thread_id: ThreadId,
        reason: String,
        reply: Reply<ThreadProjection>,
    },
    Drive {
        thread_id: ThreadId,
        harness_hint: Option<String>,
        max_steps: u64,
        reply: Reply<KernelDrive>,
    },
    Advance {
        thread_id: ThreadId,
        harness_hint: Option<String>,
        reply: Reply<ThreadProjection>,
    },
    DispatchScope {
        thread_id: ThreadId,
        scope_id: ScopeId,
        harness_hint: Option<String>,
        reply: Reply<ThreadProjection>,
    },
    SpawnSubagent {
        thread_id: ThreadId,
        parent_scope: ScopeId,
        task: String,
        max_autonomous_effect: SideEffectKind,
        depends_on: Vec<ScopeId>,
        reply: Reply<ScopeId>,
    },
    PlanHistory {
        thread_id: ThreadId,
        reply: Reply<Vec<Plan>>,
    },
    AnalyzePlanImpact {
        thread_id: ThreadId,
        source_event_id: String,
        source: String,
        summary: String,
        reply: Reply<Option<PlanImpact>>,
    },
    MemoryProjection {
        reply: Reply<MemoryProjection>,
    },
    PromoteMemory {
        memory_id: MemoryId,
        scope: Option<MemoryScope>,
        reply: Reply<MemoryRecord>,
    },
    RevokeMemory {
        memory_id: MemoryId,
        reason: String,
        reply: Reply<()>,
    },
    CorrectMemory {
        scope: MemoryScope,
        subject: String,
        predicate: String,
        value: String,
        reply: Reply<MemoryRecord>,
    },
    Checkpoint {
        thread_id: ThreadId,
        reply: Reply<else_protocol::artifact::Checkpoint>,
    },
    Rerun {
        thread_id: ThreadId,
        checkpoint_id: CheckpointId,
        alternative: bool,
        reply: Reply<(ScopeId, u64)>,
    },
    Compare {
        thread_id: ThreadId,
        reply: Reply<String>,
    },
    ConfigureOpaqueHarness {
        thread_id: ThreadId,
        policy: OpaqueHarnessPolicy,
        reply: Reply<ThreadSpec>,
    },
    Resources {
        thread_id: ThreadId,
        reply: Reply<ResourceReport>,
    },
    ReviseThread {
        thread_id: ThreadId,
        update: ThreadRevision,
        unaffected_scopes: Vec<ScopeId>,
        reply: Reply<ThreadSpec>,
    },
    Shutdown {
        reply: Reply<()>,
    },
}

#[derive(Clone)]
pub struct RuntimeHandle {
    sender: mpsc::Sender<Command>,
}

impl RuntimeHandle {
    pub fn create_thread(
        &self,
        outcome: &str,
        project: Option<String>,
        mode: else_protocol::thread::ThreadMode,
    ) -> Result<ThreadSpec> {
        self.request(|reply| Command::CreateThread {
            outcome: outcome.to_string(),
            project,
            mode,
            reply,
        })
    }

    pub fn projection(&self, thread_id: &ThreadId) -> Result<ThreadProjection> {
        self.request(|reply| Command::Projection {
            thread_id: thread_id.clone(),
            reply,
        })
    }

    pub fn list_threads(&self) -> Result<Vec<ThreadSpec>> {
        self.request(|reply| Command::ListThreads { reply })
    }

    pub fn events(&self, thread_id: &ThreadId) -> Result<Vec<StoredEvent>> {
        self.request(|reply| Command::Events {
            thread_id: thread_id.clone(),
            reply,
        })
    }

    pub fn export_data(&self) -> Result<RuntimeDataExport> {
        self.request(|reply| Command::ExportData { reply })
    }

    pub fn delete_thread(&self, thread_id: &ThreadId) -> Result<ThreadDeletionReport> {
        self.request(|reply| Command::DeleteThread {
            thread_id: thread_id.clone(),
            reply,
        })
    }

    pub fn retry_failed_scopes(
        &self,
        thread_id: &ThreadId,
    ) -> Result<(Vec<ScopeId>, ThreadProjection)> {
        self.request(|reply| Command::RetryFailedScopes {
            thread_id: thread_id.clone(),
            reply,
        })
    }

    pub fn runtime_settings(&self) -> Result<RuntimeSettings> {
        self.request(|reply| Command::GetRuntimeSettings { reply })
    }

    pub fn update_runtime_settings(&self, settings: RuntimeSettings) -> Result<RuntimeSettings> {
        self.request(|reply| Command::UpdateRuntimeSettings { settings, reply })
    }

    pub fn diagnostics(&self) -> Result<RuntimeDiagnosticReport> {
        self.request(|reply| Command::Diagnostics { reply })
    }

    pub fn cleanup_orphan_leases(&self) -> Result<LeaseCleanupReport> {
        self.request(|reply| Command::CleanupOrphanLeases { reply })
    }

    pub fn capabilities(&self, thread_id: &ThreadId) -> Result<Vec<CapabilityRegistration>> {
        self.request(|reply| Command::Capabilities {
            thread_id: thread_id.clone(),
            reply,
        })
    }

    pub fn agent_bindings(&self, thread_id: &ThreadId) -> Result<Vec<AgentBinding>> {
        self.request(|reply| Command::Bindings {
            thread_id: thread_id.clone(),
            reply,
        })
    }

    pub fn register_capability(
        &self,
        thread_id: &ThreadId,
        capability: CapabilityRegistration,
    ) -> Result<()> {
        self.request(|reply| Command::RegisterCapability {
            thread_id: thread_id.clone(),
            capability,
            reply,
        })
    }

    pub fn discover_mcp_tools(
        &self,
        thread_id: &ThreadId,
        server_name: &str,
        command: &str,
    ) -> Result<Vec<CapabilityRegistration>> {
        self.request(|reply| Command::DiscoverMcp {
            thread_id: thread_id.clone(),
            server_name: server_name.into(),
            command: command.into(),
            reply,
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn bind_agent(
        &self,
        thread_id: &ThreadId,
        agent_id: AgentId,
        name: &str,
        scope_id: ScopeId,
        run_id: Option<RunId>,
        outbound: AgentTransport,
        capability_ids: Vec<CapabilityId>,
        ttl_seconds: i64,
    ) -> Result<AgentBinding> {
        self.request(|reply| Command::BindAgent {
            thread_id: thread_id.clone(),
            agent_id,
            name: name.into(),
            scope_id,
            run_id,
            outbound,
            capability_ids,
            ttl_seconds,
            reply,
        })
    }

    pub fn revoke_binding(
        &self,
        thread_id: &ThreadId,
        binding_id: &AgentBindingId,
        reason: &str,
    ) -> Result<()> {
        self.request(|reply| Command::RevokeBinding {
            thread_id: thread_id.clone(),
            binding_id: binding_id.clone(),
            reason: reason.into(),
            reply,
        })
    }

    pub fn dispatch_agent_binding(
        &self,
        thread_id: &ThreadId,
        binding_id: &AgentBindingId,
    ) -> Result<ThreadProjection> {
        self.request(|reply| Command::DispatchBinding {
            thread_id: thread_id.clone(),
            binding_id: binding_id.clone(),
            reply,
        })
    }

    pub fn record_agent_binding_event(
        &self,
        thread_id: &ThreadId,
        binding_id: &AgentBindingId,
        session: &str,
        event: AgentBindingEvent,
    ) -> Result<ThreadProjection> {
        self.request(|reply| Command::RecordBindingEvent {
            thread_id: thread_id.clone(),
            binding_id: binding_id.clone(),
            session: session.into(),
            event,
            reply,
        })
    }

    pub fn transition_agent_binding(
        &self,
        thread_id: &ThreadId,
        binding_id: &AgentBindingId,
        state: AgentBindingState,
    ) -> Result<()> {
        self.request(|reply| Command::TransitionBinding {
            thread_id: thread_id.clone(),
            binding_id: binding_id.clone(),
            state,
            reply,
        })
    }

    pub fn register_calendar_connector(
        &self,
        thread_id: &ThreadId,
        name: &str,
        source: &str,
    ) -> Result<ConnectorRegistration> {
        self.request(|reply| Command::RegisterCalendar {
            thread_id: thread_id.clone(),
            name: name.into(),
            source: source.into(),
            reply,
        })
    }

    pub fn register_linear_connector(
        &self,
        thread_id: &ThreadId,
        name: &str,
        source: &str,
    ) -> Result<ConnectorRegistration> {
        self.request(|reply| Command::RegisterLinear {
            thread_id: thread_id.clone(),
            name: name.into(),
            source: source.into(),
            reply,
        })
    }

    pub fn register_connector_adapter(
        &self,
        thread_id: &ThreadId,
        name: &str,
        source: &str,
        adapter: &str,
    ) -> Result<ConnectorRegistration> {
        self.register_connector_adapter_with_credential(thread_id, name, source, adapter, None, 300)
    }

    pub fn register_connector_adapter_with_credential(
        &self,
        thread_id: &ThreadId,
        name: &str,
        source: &str,
        adapter: &str,
        credential: Option<ConnectorCredential>,
        sync_interval_seconds: u64,
    ) -> Result<ConnectorRegistration> {
        self.request(|reply| Command::RegisterConnectorAdapter {
            thread_id: thread_id.clone(),
            name: name.into(),
            source: source.into(),
            adapter: adapter.into(),
            credential,
            sync_interval_seconds,
            reply,
        })
    }

    pub fn attach_connector_credential(
        &self,
        thread_id: &ThreadId,
        connector_id: &ConnectorId,
        credential: ConnectorCredential,
    ) -> Result<ConnectorCredentialMetadata> {
        self.request(|reply| Command::AttachConnectorCredential {
            thread_id: thread_id.clone(),
            connector_id: connector_id.clone(),
            credential,
            reply,
        })
    }

    pub fn revoke_connector_credential(
        &self,
        thread_id: &ThreadId,
        connector_id: &ConnectorId,
    ) -> Result<ConnectorCredentialMetadata> {
        self.request(|reply| Command::RevokeConnectorCredential {
            thread_id: thread_id.clone(),
            connector_id: connector_id.clone(),
            reply,
        })
    }

    pub fn connector_health(
        &self,
        thread_id: &ThreadId,
        connector_id: &ConnectorId,
    ) -> Result<ConnectorHealthReport> {
        self.request(|reply| Command::ConnectorHealth {
            thread_id: thread_id.clone(),
            connector_id: connector_id.clone(),
            reply,
        })
    }

    pub fn propose_connector_action(
        &self,
        thread_id: &ThreadId,
        connector_id: &ConnectorId,
        capability: &str,
        target: &str,
        input: serde_json::Value,
        idempotency_key: Option<String>,
    ) -> Result<ConnectorAction> {
        self.request(|reply| Command::ProposeConnectorAction {
            thread_id: thread_id.clone(),
            connector_id: connector_id.clone(),
            capability: capability.into(),
            target: target.into(),
            input,
            idempotency_key,
            reply,
        })
    }

    pub fn preview_connector_action(
        &self,
        thread_id: &ThreadId,
        connector_id: &ConnectorId,
        capability: &str,
        target: &str,
        input: serde_json::Value,
    ) -> Result<ConnectorActionPreview> {
        self.request(|reply| Command::PreviewConnectorAction {
            thread_id: thread_id.clone(),
            connector_id: connector_id.clone(),
            capability: capability.into(),
            target: target.into(),
            input,
            reply,
        })
    }

    pub fn execute_connector_action(
        &self,
        thread_id: &ThreadId,
        action_id: &ConnectorActionId,
    ) -> Result<ConnectorAction> {
        self.request(|reply| Command::ExecuteConnectorAction {
            thread_id: thread_id.clone(),
            action_id: action_id.clone(),
            reply,
        })
    }

    pub fn connector_actions(&self, thread_id: &ThreadId) -> Result<Vec<ConnectorAction>> {
        self.request(|reply| Command::ConnectorActions {
            thread_id: thread_id.clone(),
            reply,
        })
    }

    pub fn verify_connector_action(
        &self,
        thread_id: &ThreadId,
        action_id: &ConnectorActionId,
    ) -> Result<ConnectorAction> {
        self.request(|reply| Command::VerifyConnectorAction {
            thread_id: thread_id.clone(),
            action_id: action_id.clone(),
            reply,
        })
    }

    pub fn compensate_connector_action(
        &self,
        thread_id: &ThreadId,
        action_id: &ConnectorActionId,
    ) -> Result<ConnectorAction> {
        self.request(|reply| Command::CompensateConnectorAction {
            thread_id: thread_id.clone(),
            action_id: action_id.clone(),
            reply,
        })
    }

    pub fn connector_manifests(&self) -> Result<Vec<ConnectorManifest>> {
        self.request(|reply| Command::ConnectorAdapters { reply })
    }

    pub fn sync_connector(
        &self,
        thread_id: &ThreadId,
        connector_id: &ConnectorId,
    ) -> Result<ConnectorSyncResult> {
        self.request(|reply| Command::SyncConnector {
            thread_id: thread_id.clone(),
            connector_id: connector_id.clone(),
            reply,
        })
    }

    pub fn repair_connector(
        &self,
        thread_id: &ThreadId,
        connector_id: &ConnectorId,
    ) -> Result<ConnectorRepairReport> {
        self.request(|reply| Command::RepairConnector {
            thread_id: thread_id.clone(),
            connector_id: connector_id.clone(),
            reply,
        })
    }

    pub fn ingest_connector_webhook(
        &self,
        thread_id: &ThreadId,
        connector_id: &ConnectorId,
        session_token: &str,
        payload: serde_json::Value,
    ) -> Result<ConnectorSyncResult> {
        self.request(|reply| Command::IngestConnectorWebhook {
            thread_id: thread_id.clone(),
            connector_id: connector_id.clone(),
            session_token: session_token.into(),
            payload,
            reply,
        })
    }

    pub fn configure_connector_policy(
        &self,
        thread_id: &ThreadId,
        policy: ConnectorPolicy,
    ) -> Result<ThreadSpec> {
        self.request(|reply| Command::ConfigureConnectorPolicy {
            thread_id: thread_id.clone(),
            policy,
            reply,
        })
    }

    pub fn world_signals(
        &self,
        cursor: u64,
        provider: Option<&str>,
        entity: Option<&str>,
    ) -> Result<Vec<else_protocol::connector::WorldSignalRecord>> {
        self.request(|reply| Command::WorldSignals {
            cursor,
            provider: provider.map(str::to_string),
            entity: entity.map(str::to_string),
            reply,
        })
    }

    pub fn thread_candidates(
        &self,
        cursor: u64,
    ) -> Result<Vec<else_protocol::connector::ThreadCandidateRecord>> {
        self.request(|reply| Command::ThreadCandidates { cursor, reply })
    }

    pub fn world_items(
        &self,
        thread_id: &ThreadId,
        include_resolved: bool,
    ) -> Result<Vec<else_protocol::world::WorldItem>> {
        self.request(|reply| Command::WorldItems {
            thread_id: thread_id.clone(),
            include_resolved,
            reply,
        })
    }

    pub fn personal_brief(&self) -> Result<else_protocol::world::PersonalBrief> {
        self.request(|reply| Command::PersonalBrief { reply })
    }

    pub fn resolve_world_item(
        &self,
        thread_id: &ThreadId,
        item_id: &else_protocol::ids::WorldItemId,
        kind: else_protocol::world::WorldItemResolutionKind,
        note: Option<String>,
    ) -> Result<else_protocol::world::WorldItem> {
        self.request(|reply| Command::ResolveWorldItem {
            thread_id: thread_id.clone(),
            item_id: item_id.clone(),
            kind,
            note,
            reply,
        })
    }

    pub fn correct_world_correlation(
        &self,
        thread_id: &ThreadId,
        correction: else_protocol::world::WorldCorrelationCorrection,
    ) -> Result<else_protocol::world::WorldCorrelationCorrection> {
        self.request(|reply| Command::CorrectWorldCorrelation {
            thread_id: thread_id.clone(),
            correction,
            reply,
        })
    }

    pub fn project_worlds(
        &self,
        thread_id: Option<&ThreadId>,
        include_resolved: bool,
    ) -> Result<Vec<else_protocol::world::ProjectWorld>> {
        self.request(|reply| Command::ProjectWorlds {
            thread_id: thread_id.cloned(),
            include_resolved,
            reply,
        })
    }

    pub fn identity_worlds(
        &self,
        thread_id: Option<&ThreadId>,
        include_resolved: bool,
    ) -> Result<Vec<else_protocol::world::IdentityWorld>> {
        self.request(|reply| Command::IdentityWorlds {
            thread_id: thread_id.cloned(),
            include_resolved,
            reply,
        })
    }

    pub fn resolve_signals(&self, thread_id: &ThreadId) -> Result<Vec<ThreadCandidate>> {
        self.request(|reply| Command::ResolveSignals {
            thread_id: thread_id.clone(),
            reply,
        })
    }

    pub fn propose_thread_candidate(
        &self,
        thread_id: &ThreadId,
        candidate: ThreadCandidate,
    ) -> Result<ThreadCandidate> {
        self.request(|reply| Command::ProposeCandidate {
            thread_id: thread_id.clone(),
            candidate,
            reply,
        })
    }

    pub fn resolve_thread_candidate(
        &self,
        thread_id: &ThreadId,
        candidate_id: &ThreadCandidateId,
        accept: bool,
    ) -> Result<Option<ThreadSpec>> {
        self.request(|reply| Command::ResolveCandidate {
            thread_id: thread_id.clone(),
            candidate_id: candidate_id.clone(),
            accept,
            reply,
        })
    }

    pub fn record_external_event(
        &self,
        thread_id: &ThreadId,
        source: &str,
        summary: &str,
    ) -> Result<ThreadProjection> {
        self.request(|reply| Command::RecordExternalEvent {
            thread_id: thread_id.clone(),
            source: source.to_string(),
            summary: summary.to_string(),
            reply,
        })
    }

    pub fn wait_for_external(
        &self,
        thread_id: &ThreadId,
        reason: &str,
    ) -> Result<ThreadProjection> {
        self.request(|reply| Command::WaitForExternal {
            thread_id: thread_id.clone(),
            reason: reason.to_string(),
            reply,
        })
    }

    pub fn schedule_timer(
        &self,
        thread_id: &ThreadId,
        fire_at: DateTime<Utc>,
        reason: &str,
    ) -> Result<TimerId> {
        self.request(|reply| Command::ScheduleTimer {
            thread_id: thread_id.clone(),
            fire_at,
            reason: reason.to_string(),
            reply,
        })
    }

    pub fn decide(
        &self,
        thread_id: &ThreadId,
        request_id: &AttentionId,
        choice: DecisionChoice,
    ) -> Result<ThreadProjection> {
        self.request(|reply| Command::Decide {
            thread_id: thread_id.clone(),
            request_id: request_id.clone(),
            choice,
            reply,
        })
    }

    pub fn submit_attention_request(
        &self,
        thread_id: &ThreadId,
        scope_id: Option<ScopeId>,
        request: AttentionRequest,
    ) -> Result<AttentionId> {
        self.request(|reply| Command::SubmitAttention {
            thread_id: thread_id.clone(),
            scope_id,
            request,
            reply,
        })
    }

    pub fn propose_plan(&self, thread_id: &ThreadId, plan: Plan) -> Result<Plan> {
        self.request(|reply| Command::ProposePlan {
            thread_id: thread_id.clone(),
            plan,
            reply,
        })
    }

    pub fn abort(&self, thread_id: &ThreadId, reason: &str) -> Result<ThreadProjection> {
        self.request(|reply| Command::Abort {
            thread_id: thread_id.clone(),
            reason: reason.to_string(),
            reply,
        })
    }

    pub fn drive(
        &self,
        thread_id: &ThreadId,
        harness_hint: Option<&str>,
        max_steps: u64,
    ) -> Result<KernelDrive> {
        self.request(|reply| Command::Drive {
            thread_id: thread_id.clone(),
            harness_hint: harness_hint.map(str::to_string),
            max_steps,
            reply,
        })
    }

    pub fn advance(
        &self,
        thread_id: &ThreadId,
        harness_hint: Option<&str>,
    ) -> Result<ThreadProjection> {
        self.request(|reply| Command::Advance {
            thread_id: thread_id.clone(),
            harness_hint: harness_hint.map(str::to_string),
            reply,
        })
    }

    pub fn dispatch_scope(
        &self,
        thread_id: &ThreadId,
        scope_id: &ScopeId,
        harness_hint: Option<&str>,
    ) -> Result<ThreadProjection> {
        self.request(|reply| Command::DispatchScope {
            thread_id: thread_id.clone(),
            scope_id: scope_id.clone(),
            harness_hint: harness_hint.map(str::to_string),
            reply,
        })
    }

    pub fn spawn_subagent_with_dependencies(
        &self,
        thread_id: &ThreadId,
        parent_scope: &ScopeId,
        task: &str,
        max_autonomous_effect: SideEffectKind,
        depends_on: Vec<ScopeId>,
    ) -> Result<ScopeId> {
        self.request(|reply| Command::SpawnSubagent {
            thread_id: thread_id.clone(),
            parent_scope: parent_scope.clone(),
            task: task.to_string(),
            max_autonomous_effect,
            depends_on,
            reply,
        })
    }

    pub fn plan_history(&self, thread_id: &ThreadId) -> Result<Vec<Plan>> {
        self.request(|reply| Command::PlanHistory {
            thread_id: thread_id.clone(),
            reply,
        })
    }

    pub fn analyze_plan_impact(
        &self,
        thread_id: &ThreadId,
        source_event_id: &str,
        source: &str,
        summary: &str,
    ) -> Result<Option<PlanImpact>> {
        self.request(|reply| Command::AnalyzePlanImpact {
            thread_id: thread_id.clone(),
            source_event_id: source_event_id.to_string(),
            source: source.to_string(),
            summary: summary.to_string(),
            reply,
        })
    }

    pub fn memory_projection(&self) -> Result<MemoryProjection> {
        self.request(|reply| Command::MemoryProjection { reply })
    }

    pub fn promote_memory(
        &self,
        memory_id: &MemoryId,
        scope: Option<MemoryScope>,
    ) -> Result<MemoryRecord> {
        self.request(|reply| Command::PromoteMemory {
            memory_id: memory_id.clone(),
            scope,
            reply,
        })
    }

    pub fn revoke_memory(&self, memory_id: &MemoryId, reason: &str) -> Result<()> {
        self.request(|reply| Command::RevokeMemory {
            memory_id: memory_id.clone(),
            reason: reason.to_string(),
            reply,
        })
    }

    pub fn correct_memory(
        &self,
        scope: MemoryScope,
        subject: &str,
        predicate: &str,
        value: &str,
    ) -> Result<MemoryRecord> {
        self.request(|reply| Command::CorrectMemory {
            scope,
            subject: subject.to_string(),
            predicate: predicate.to_string(),
            value: value.to_string(),
            reply,
        })
    }

    pub fn checkpoint(&self, thread_id: &ThreadId) -> Result<else_protocol::artifact::Checkpoint> {
        self.request(|reply| Command::Checkpoint {
            thread_id: thread_id.clone(),
            reply,
        })
    }

    pub fn rerun(
        &self,
        thread_id: &ThreadId,
        checkpoint_id: &CheckpointId,
        alternative: bool,
    ) -> Result<(ScopeId, u64)> {
        self.request(|reply| Command::Rerun {
            thread_id: thread_id.clone(),
            checkpoint_id: checkpoint_id.clone(),
            alternative,
            reply,
        })
    }

    pub fn compare(&self, thread_id: &ThreadId) -> Result<String> {
        self.request(|reply| Command::Compare {
            thread_id: thread_id.clone(),
            reply,
        })
    }

    pub fn configure_opaque_harness(
        &self,
        thread_id: &ThreadId,
        policy: OpaqueHarnessPolicy,
    ) -> Result<ThreadSpec> {
        self.request(|reply| Command::ConfigureOpaqueHarness {
            thread_id: thread_id.clone(),
            policy,
            reply,
        })
    }

    pub fn resources(&self, thread_id: &ThreadId) -> Result<ResourceReport> {
        self.request(|reply| Command::Resources {
            thread_id: thread_id.clone(),
            reply,
        })
    }

    pub fn revise_thread(
        &self,
        thread_id: &ThreadId,
        update: ThreadRevision,
        unaffected_scopes: &[ScopeId],
    ) -> Result<ThreadSpec> {
        self.request(|reply| Command::ReviseThread {
            thread_id: thread_id.clone(),
            update,
            unaffected_scopes: unaffected_scopes.to_vec(),
            reply,
        })
    }

    pub fn shutdown(&self) -> Result<()> {
        self.request(|reply| Command::Shutdown { reply })
    }

    fn request<T>(&self, make: impl FnOnce(Reply<T>) -> Command) -> Result<T> {
        let (reply, response) = mpsc::sync_channel(1);
        self.sender
            .send(make(reply))
            .map_err(|_| ApiError::WorkerStopped)?;
        response.recv().map_err(|_| ApiError::WorkerStopped)?
    }
}

pub struct RuntimeService {
    handle: RuntimeHandle,
    worker: Option<JoinHandle<()>>,
}

impl RuntimeService {
    pub fn start(dir: &Path) -> Result<Self> {
        let runtime = Runtime::open(dir)?;
        let _ = runtime.restore_mcp_sessions();
        let (sender, receiver) = mpsc::channel();
        let worker = thread::spawn(move || worker_loop(runtime, receiver));
        Ok(Self {
            handle: RuntimeHandle { sender },
            worker: Some(worker),
        })
    }

    pub fn handle(&self) -> RuntimeHandle {
        self.handle.clone()
    }

    pub fn shutdown(mut self) {
        self.stop();
    }

    fn stop(&mut self) {
        let _ = self.handle.shutdown();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

impl Drop for RuntimeService {
    fn drop(&mut self) {
        self.stop();
    }
}

fn worker_loop(runtime: Runtime, receiver: mpsc::Receiver<Command>) {
    loop {
        let wake_at = reconcile_all(&runtime);
        let timeout = wait_duration(wake_at);
        match receiver.recv_timeout(timeout) {
            Ok(Command::Shutdown { reply }) => {
                let _ = reply.send(Ok(()));
                break;
            }
            Ok(command) => handle_command(&runtime, command),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }
}

fn handle_command(runtime: &Runtime, command: Command) {
    match command {
        Command::CreateThread {
            outcome,
            project,
            mode,
            reply,
        } => {
            let _ = reply.send(runtime.create_thread(&outcome, project, mode));
        }
        Command::Projection { thread_id, reply } => {
            let _ = reply.send(runtime.projection(&thread_id));
        }
        Command::ListThreads { reply } => {
            let _ = reply.send(runtime.list_threads());
        }
        Command::Events { thread_id, reply } => {
            let _ = reply.send(runtime.events(&thread_id));
        }
        Command::ExportData { reply } => {
            let _ = reply.send(runtime.export_data());
        }
        Command::DeleteThread { thread_id, reply } => {
            let _ = reply.send(runtime.delete_thread(&thread_id));
        }
        Command::RetryFailedScopes { thread_id, reply } => {
            let _ = reply.send(runtime.retry_failed_scopes(&thread_id));
        }
        Command::GetRuntimeSettings { reply } => {
            let _ = reply.send(runtime.runtime_settings());
        }
        Command::UpdateRuntimeSettings { settings, reply } => {
            let _ = reply.send(runtime.update_runtime_settings(settings));
        }
        Command::Diagnostics { reply } => {
            let _ = reply.send(runtime.diagnostics());
        }
        Command::CleanupOrphanLeases { reply } => {
            let _ = reply.send(runtime.cleanup_orphan_leases());
        }
        Command::Capabilities { thread_id, reply } => {
            let _ = reply.send(runtime.capabilities(&thread_id));
        }
        Command::Bindings { thread_id, reply } => {
            let _ = reply.send(runtime.agent_bindings(&thread_id));
        }
        Command::RegisterCapability {
            thread_id,
            capability,
            reply,
        } => {
            let _ = reply.send(runtime.register_capability(&thread_id, capability));
        }
        Command::DiscoverMcp {
            thread_id,
            server_name,
            command,
            reply,
        } => {
            let _ = reply.send(runtime.discover_mcp_tools(&thread_id, &server_name, &command));
        }
        Command::BindAgent {
            thread_id,
            agent_id,
            name,
            scope_id,
            run_id,
            outbound,
            capability_ids,
            ttl_seconds,
            reply,
        } => {
            let _ = reply.send(runtime.bind_agent(
                &thread_id,
                agent_id,
                &name,
                scope_id,
                run_id,
                outbound,
                capability_ids,
                ttl_seconds,
            ));
        }
        Command::RevokeBinding {
            thread_id,
            binding_id,
            reason,
            reply,
        } => {
            let _ = reply.send(runtime.revoke_agent_binding(&thread_id, &binding_id, &reason));
        }
        Command::DispatchBinding {
            thread_id,
            binding_id,
            reply,
        } => {
            let _ = reply.send(runtime.dispatch_agent_binding(&thread_id, &binding_id));
        }
        Command::RecordBindingEvent {
            thread_id,
            binding_id,
            session,
            event,
            reply,
        } => {
            let _ = reply.send(runtime.record_agent_binding_event(
                &thread_id,
                &binding_id,
                &session,
                event,
            ));
        }
        Command::TransitionBinding {
            thread_id,
            binding_id,
            state,
            reply,
        } => {
            let _ = reply.send(runtime.transition_agent_binding(&thread_id, &binding_id, state));
        }
        Command::RegisterCalendar {
            thread_id,
            name,
            source,
            reply,
        } => {
            let _ = reply.send(runtime.register_calendar_connector(&thread_id, &name, &source));
        }
        Command::RegisterLinear {
            thread_id,
            name,
            source,
            reply,
        } => {
            let _ = reply.send(runtime.register_linear_connector(&thread_id, &name, &source));
        }
        Command::RegisterConnectorAdapter {
            thread_id,
            name,
            source,
            adapter,
            credential,
            sync_interval_seconds,
            reply,
        } => {
            let _ = reply.send(runtime.register_connector_adapter_with_credential(
                &thread_id,
                &name,
                &source,
                &adapter,
                credential,
                sync_interval_seconds,
            ));
        }
        Command::AttachConnectorCredential {
            thread_id,
            connector_id,
            credential,
            reply,
        } => {
            let _ = reply.send(runtime.attach_connector_credential(
                &thread_id,
                &connector_id,
                credential,
            ));
        }
        Command::RevokeConnectorCredential {
            thread_id,
            connector_id,
            reply,
        } => {
            let _ = reply.send(runtime.revoke_connector_credential(&thread_id, &connector_id));
        }
        Command::ConnectorHealth {
            thread_id,
            connector_id,
            reply,
        } => {
            let _ = reply.send(runtime.connector_health(&thread_id, &connector_id));
        }
        Command::PreviewConnectorAction {
            thread_id,
            connector_id,
            capability,
            target,
            input,
            reply,
        } => {
            let _ = reply.send(runtime.preview_connector_action(
                &thread_id,
                &connector_id,
                &capability,
                &target,
                &input,
            ));
        }
        Command::ProposeConnectorAction {
            thread_id,
            connector_id,
            capability,
            target,
            input,
            idempotency_key,
            reply,
        } => {
            let _ = reply.send(runtime.propose_connector_action(
                &thread_id,
                &connector_id,
                &capability,
                &target,
                input,
                idempotency_key,
            ));
        }
        Command::ExecuteConnectorAction {
            thread_id,
            action_id,
            reply,
        } => {
            let _ = reply.send(runtime.execute_connector_action(&thread_id, &action_id));
        }
        Command::VerifyConnectorAction {
            thread_id,
            action_id,
            reply,
        } => {
            let _ = reply.send(runtime.verify_connector_action(&thread_id, &action_id));
        }
        Command::CompensateConnectorAction {
            thread_id,
            action_id,
            reply,
        } => {
            let _ = reply.send(runtime.compensate_connector_action(&thread_id, &action_id));
        }
        Command::ConnectorActions { thread_id, reply } => {
            let _ = reply.send(runtime.connector_actions(&thread_id));
        }
        Command::ConnectorAdapters { reply } => {
            let _ = reply.send(Ok(runtime.connector_manifests()));
        }
        Command::SyncConnector {
            thread_id,
            connector_id,
            reply,
        } => {
            let _ = reply.send(runtime.sync_connector(&thread_id, &connector_id));
        }
        Command::RepairConnector {
            thread_id,
            connector_id,
            reply,
        } => {
            let _ = reply.send(runtime.repair_connector(&thread_id, &connector_id));
        }
        Command::IngestConnectorWebhook {
            thread_id,
            connector_id,
            session_token,
            payload,
            reply,
        } => {
            let _ = reply.send(runtime.ingest_connector_webhook(
                &thread_id,
                &connector_id,
                &session_token,
                payload,
            ));
        }
        Command::ConfigureConnectorPolicy {
            thread_id,
            policy,
            reply,
        } => {
            let _ = reply.send(runtime.configure_connector_policy(&thread_id, policy));
        }
        Command::WorldSignals {
            cursor,
            provider,
            entity,
            reply,
        } => {
            let _ =
                reply.send(runtime.world_signals(cursor, provider.as_deref(), entity.as_deref()));
        }
        Command::ThreadCandidates { cursor, reply } => {
            let _ = reply.send(runtime.thread_candidates(cursor));
        }
        Command::WorldItems {
            thread_id,
            include_resolved,
            reply,
        } => {
            let _ = reply.send(runtime.world_items(&thread_id, include_resolved));
        }
        Command::PersonalBrief { reply } => {
            let _ = reply.send(runtime.personal_brief());
        }
        Command::ResolveWorldItem {
            thread_id,
            item_id,
            kind,
            note,
            reply,
        } => {
            let _ = reply.send(runtime.resolve_world_item(&thread_id, &item_id, kind, note));
        }
        Command::CorrectWorldCorrelation {
            thread_id,
            correction,
            reply,
        } => {
            let _ = reply.send(runtime.correct_world_correlation(&thread_id, correction));
        }
        Command::ProjectWorlds {
            thread_id,
            include_resolved,
            reply,
        } => {
            let _ = reply.send(runtime.project_worlds(thread_id.as_ref(), include_resolved));
        }
        Command::IdentityWorlds {
            thread_id,
            include_resolved,
            reply,
        } => {
            let _ = reply.send(runtime.identity_worlds(thread_id.as_ref(), include_resolved));
        }
        Command::ResolveSignals { thread_id, reply } => {
            let _ = reply.send(runtime.resolve_signals(&thread_id));
        }
        Command::ProposeCandidate {
            thread_id,
            candidate,
            reply,
        } => {
            let _ = reply.send(runtime.propose_thread_candidate(&thread_id, candidate));
        }
        Command::ResolveCandidate {
            thread_id,
            candidate_id,
            accept,
            reply,
        } => {
            let _ = reply.send(runtime.resolve_thread_candidate(&thread_id, &candidate_id, accept));
        }
        Command::RecordExternalEvent {
            thread_id,
            source,
            summary,
            reply,
        } => {
            let _ = reply.send(runtime.record_external_event(&thread_id, &source, &summary));
        }
        Command::WaitForExternal {
            thread_id,
            reason,
            reply,
        } => {
            let _ = reply.send(runtime.wait_for_external(&thread_id, &reason));
        }
        Command::ScheduleTimer {
            thread_id,
            fire_at,
            reason,
            reply,
        } => {
            let _ = reply.send(runtime.schedule_timer(&thread_id, fire_at, &reason));
        }
        Command::Decide {
            thread_id,
            request_id,
            choice,
            reply,
        } => {
            let _ = reply.send(runtime.decide(&thread_id, &request_id, choice));
        }
        Command::SubmitAttention {
            thread_id,
            scope_id,
            request,
            reply,
        } => {
            let result = match scope_id {
                Some(scope_id) => {
                    runtime.submit_attention_request_for_scope(&thread_id, &scope_id, &request)
                }
                None => runtime.submit_attention_request(&thread_id, &request),
            };
            let _ = reply.send(result);
        }
        Command::ProposePlan {
            thread_id,
            plan,
            reply,
        } => {
            let _ = reply.send(runtime.propose_plan(&thread_id, plan));
        }
        Command::Abort {
            thread_id,
            reason,
            reply,
        } => {
            let _ = reply.send(runtime.abort(&thread_id, &reason));
        }
        Command::Drive {
            thread_id,
            harness_hint,
            max_steps,
            reply,
        } => {
            let _ = reply.send(runtime.drive(&thread_id, harness_hint.as_deref(), max_steps));
        }
        Command::Advance {
            thread_id,
            harness_hint,
            reply,
        } => {
            let _ = reply.send(runtime.advance(&thread_id, harness_hint.as_deref()));
        }
        Command::DispatchScope {
            thread_id,
            scope_id,
            harness_hint,
            reply,
        } => {
            let _ =
                reply.send(runtime.dispatch_scope(&thread_id, &scope_id, harness_hint.as_deref()));
        }
        Command::SpawnSubagent {
            thread_id,
            parent_scope,
            task,
            max_autonomous_effect,
            depends_on,
            reply,
        } => {
            let _ = reply.send(runtime.spawn_subagent_with_dependencies(
                &thread_id,
                &parent_scope,
                &task,
                max_autonomous_effect,
                depends_on,
            ));
        }
        Command::PlanHistory { thread_id, reply } => {
            let _ = reply.send(runtime.plan_history(&thread_id));
        }
        Command::AnalyzePlanImpact {
            thread_id,
            source_event_id,
            source,
            summary,
            reply,
        } => {
            let _ = reply.send(runtime.analyze_plan_impact(
                &thread_id,
                &source_event_id,
                &source,
                &summary,
            ));
        }
        Command::MemoryProjection { reply } => {
            let _ = reply.send(runtime.memory_projection());
        }
        Command::PromoteMemory {
            memory_id,
            scope,
            reply,
        } => {
            let _ = reply.send(runtime.promote_memory(&memory_id, scope));
        }
        Command::RevokeMemory {
            memory_id,
            reason,
            reply,
        } => {
            let _ = reply.send(runtime.revoke_memory(&memory_id, &reason));
        }
        Command::CorrectMemory {
            scope,
            subject,
            predicate,
            value,
            reply,
        } => {
            let _ = reply.send(runtime.correct_memory(scope, &subject, &predicate, &value));
        }
        Command::Checkpoint { thread_id, reply } => {
            let _ = reply.send(runtime.checkpoint(&thread_id));
        }
        Command::Rerun {
            thread_id,
            checkpoint_id,
            alternative,
            reply,
        } => {
            let _ = reply.send(runtime.rerun(&thread_id, &checkpoint_id, alternative));
        }
        Command::Compare { thread_id, reply } => {
            let _ = reply.send(runtime.compare(&thread_id));
        }
        Command::ConfigureOpaqueHarness {
            thread_id,
            policy,
            reply,
        } => {
            let _ = reply.send(runtime.configure_opaque_harness(&thread_id, policy));
        }
        Command::Resources { thread_id, reply } => {
            let _ = reply.send(runtime.resources(&thread_id));
        }
        Command::ReviseThread {
            thread_id,
            update,
            unaffected_scopes,
            reply,
        } => {
            let _ = reply.send(runtime.revise_thread(&thread_id, update, &unaffected_scopes));
        }
        Command::Shutdown { reply } => {
            let _ = reply.send(Ok(()));
        }
    }
}

fn reconcile_all(runtime: &Runtime) -> Option<DateTime<Utc>> {
    let _ = runtime.enforce_retention(Utc::now());
    let _ = runtime.refresh_dirty_mcp_servers();
    let mut next_wake = runtime.sync_due_connectors(Utc::now());
    let Ok(threads) = runtime.list_threads() else {
        return Some(Utc::now() + chrono::Duration::seconds(2));
    };
    let mut had_error = false;
    for spec in threads {
        let Ok(projection) = runtime.projection(&spec.id) else {
            had_error = true;
            continue;
        };
        if matches!(
            projection.state,
            Some(
                else_protocol::thread::ThreadState::Completed
                    | else_protocol::thread::ThreadState::Failed
                    | else_protocol::thread::ThreadState::Cancelled
            )
        ) {
            continue;
        }
        // 后台只推进调度层，不替用户启动 Run：创建与配置之间的空隙
        // 不能被 reconciler 抢跑。
        if runtime.drive_scheduling(&spec.id, 16).is_err() {
            had_error = true;
        }
        if let Ok(projection) = runtime.projection(&spec.id) {
            if let Some(due_at) = projection.next_timer_at() {
                next_wake =
                    Some(next_wake.map_or(due_at, |current: DateTime<Utc>| current.min(due_at)));
            }
        }
    }
    if had_error {
        let retry_at = Utc::now() + chrono::Duration::seconds(2);
        next_wake = Some(next_wake.map_or(retry_at, |current| current.min(retry_at)));
    }
    next_wake
}

fn wait_duration(wake_at: Option<DateTime<Utc>>) -> Duration {
    let Some(wake_at) = wake_at else {
        return Duration::from_secs(60);
    };
    let millis = (wake_at - Utc::now()).num_milliseconds().max(0) as u64;
    Duration::from_millis(millis.min(60_000))
}
