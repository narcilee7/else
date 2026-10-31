//! 常驻 Kernel supervisor：单线程持有 Runtime，命令到达或 Timer 到期时 reconcile。

use std::path::Path;
use std::sync::mpsc::{self, RecvTimeoutError, SyncSender};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use chrono::{DateTime, Utc};
use else_ledger::{IntentProjection, StoredEvent};
use else_protocol::attention::{AttentionRequest, DecisionChoice};
use else_protocol::capability::{
    AgentBinding, AgentBindingEvent, AgentBindingState, AgentTransport, CapabilityRegistration,
};
use else_protocol::connector::{
    ConnectorAction, ConnectorCredential, ConnectorCredentialMetadata, ConnectorHealthReport,
    ConnectorManifest, ConnectorPolicy, ConnectorRegistration, ConnectorSyncResult,
    IntentCandidate,
};
use else_protocol::ids::{
    AgentBindingId, AgentId, AttentionId, CapabilityId, ConnectorActionId, ConnectorId,
    IntentCandidateId, IntentId, RunId, ScopeId, TimerId,
};
use else_protocol::intent::IntentSpec;
use else_protocol::plan::Plan;

use crate::kernel::KernelDrive;
use crate::{ApiError, Result, Runtime};

type Reply<T> = SyncSender<Result<T>>;

enum Command {
    CreateIntent {
        outcome: String,
        project: Option<String>,
        reply: Reply<IntentSpec>,
    },
    Projection {
        intent_id: IntentId,
        reply: Reply<IntentProjection>,
    },
    ListIntents {
        reply: Reply<Vec<IntentSpec>>,
    },
    Events {
        intent_id: IntentId,
        reply: Reply<Vec<StoredEvent>>,
    },
    Capabilities {
        intent_id: IntentId,
        reply: Reply<Vec<CapabilityRegistration>>,
    },
    Bindings {
        intent_id: IntentId,
        reply: Reply<Vec<AgentBinding>>,
    },
    RegisterCapability {
        intent_id: IntentId,
        capability: CapabilityRegistration,
        reply: Reply<()>,
    },
    DiscoverMcp {
        intent_id: IntentId,
        server_name: String,
        command: String,
        reply: Reply<Vec<CapabilityRegistration>>,
    },
    BindAgent {
        intent_id: IntentId,
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
        intent_id: IntentId,
        binding_id: AgentBindingId,
        reason: String,
        reply: Reply<()>,
    },
    DispatchBinding {
        intent_id: IntentId,
        binding_id: AgentBindingId,
        reply: Reply<IntentProjection>,
    },
    RecordBindingEvent {
        intent_id: IntentId,
        binding_id: AgentBindingId,
        session: String,
        event: AgentBindingEvent,
        reply: Reply<IntentProjection>,
    },
    TransitionBinding {
        intent_id: IntentId,
        binding_id: AgentBindingId,
        state: AgentBindingState,
        reply: Reply<()>,
    },
    RegisterCalendar {
        intent_id: IntentId,
        name: String,
        source: String,
        reply: Reply<ConnectorRegistration>,
    },
    RegisterLinear {
        intent_id: IntentId,
        name: String,
        source: String,
        reply: Reply<ConnectorRegistration>,
    },
    RegisterConnectorAdapter {
        intent_id: IntentId,
        name: String,
        source: String,
        adapter: String,
        credential: Option<ConnectorCredential>,
        sync_interval_seconds: u64,
        reply: Reply<ConnectorRegistration>,
    },
    AttachConnectorCredential {
        intent_id: IntentId,
        connector_id: ConnectorId,
        credential: ConnectorCredential,
        reply: Reply<ConnectorCredentialMetadata>,
    },
    RevokeConnectorCredential {
        intent_id: IntentId,
        connector_id: ConnectorId,
        reply: Reply<ConnectorCredentialMetadata>,
    },
    ConnectorHealth {
        intent_id: IntentId,
        connector_id: ConnectorId,
        reply: Reply<ConnectorHealthReport>,
    },
    ProposeConnectorAction {
        intent_id: IntentId,
        connector_id: ConnectorId,
        capability: String,
        target: String,
        input: serde_json::Value,
        idempotency_key: Option<String>,
        reply: Reply<ConnectorAction>,
    },
    ExecuteConnectorAction {
        intent_id: IntentId,
        action_id: ConnectorActionId,
        reply: Reply<ConnectorAction>,
    },
    VerifyConnectorAction {
        intent_id: IntentId,
        action_id: ConnectorActionId,
        reply: Reply<ConnectorAction>,
    },
    CompensateConnectorAction {
        intent_id: IntentId,
        action_id: ConnectorActionId,
        reply: Reply<ConnectorAction>,
    },
    ConnectorActions {
        intent_id: IntentId,
        reply: Reply<Vec<ConnectorAction>>,
    },
    ConnectorAdapters {
        reply: Reply<Vec<ConnectorManifest>>,
    },
    SyncConnector {
        intent_id: IntentId,
        connector_id: ConnectorId,
        reply: Reply<ConnectorSyncResult>,
    },
    IngestConnectorWebhook {
        intent_id: IntentId,
        connector_id: ConnectorId,
        session_token: String,
        payload: serde_json::Value,
        reply: Reply<ConnectorSyncResult>,
    },
    ConfigureConnectorPolicy {
        intent_id: IntentId,
        policy: ConnectorPolicy,
        reply: Reply<IntentSpec>,
    },
    WorldSignals {
        cursor: u64,
        provider: Option<String>,
        entity: Option<String>,
        reply: Reply<Vec<else_protocol::connector::WorldSignalRecord>>,
    },
    IntentCandidates {
        cursor: u64,
        reply: Reply<Vec<else_protocol::connector::IntentCandidateRecord>>,
    },
    WorldItems {
        intent_id: IntentId,
        include_resolved: bool,
        reply: Reply<Vec<else_protocol::world::WorldItem>>,
    },
    PersonalBrief {
        reply: Reply<else_protocol::world::PersonalBrief>,
    },
    ResolveWorldItem {
        intent_id: IntentId,
        item_id: else_protocol::ids::WorldItemId,
        kind: else_protocol::world::WorldItemResolutionKind,
        note: Option<String>,
        reply: Reply<else_protocol::world::WorldItem>,
    },
    ProjectWorlds {
        intent_id: Option<IntentId>,
        include_resolved: bool,
        reply: Reply<Vec<else_protocol::world::ProjectWorld>>,
    },
    ResolveSignals {
        intent_id: IntentId,
        reply: Reply<Vec<IntentCandidate>>,
    },
    ProposeCandidate {
        intent_id: IntentId,
        candidate: IntentCandidate,
        reply: Reply<IntentCandidate>,
    },
    ResolveCandidate {
        intent_id: IntentId,
        candidate_id: IntentCandidateId,
        accept: bool,
        reply: Reply<Option<IntentSpec>>,
    },
    RecordExternalEvent {
        intent_id: IntentId,
        source: String,
        summary: String,
        reply: Reply<IntentProjection>,
    },
    WaitForExternal {
        intent_id: IntentId,
        reason: String,
        reply: Reply<IntentProjection>,
    },
    ScheduleTimer {
        intent_id: IntentId,
        fire_at: DateTime<Utc>,
        reason: String,
        reply: Reply<TimerId>,
    },
    Decide {
        intent_id: IntentId,
        request_id: AttentionId,
        choice: DecisionChoice,
        reply: Reply<IntentProjection>,
    },
    SubmitAttention {
        intent_id: IntentId,
        scope_id: Option<ScopeId>,
        request: AttentionRequest,
        reply: Reply<AttentionId>,
    },
    ProposePlan {
        intent_id: IntentId,
        plan: Plan,
        reply: Reply<Plan>,
    },
    Abort {
        intent_id: IntentId,
        reason: String,
        reply: Reply<IntentProjection>,
    },
    Drive {
        intent_id: IntentId,
        harness_hint: Option<String>,
        max_steps: u64,
        reply: Reply<KernelDrive>,
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
    pub fn create_intent(&self, outcome: &str, project: Option<String>) -> Result<IntentSpec> {
        self.request(|reply| Command::CreateIntent {
            outcome: outcome.to_string(),
            project,
            reply,
        })
    }

    pub fn projection(&self, intent_id: &IntentId) -> Result<IntentProjection> {
        self.request(|reply| Command::Projection {
            intent_id: intent_id.clone(),
            reply,
        })
    }

    pub fn list_intents(&self) -> Result<Vec<IntentSpec>> {
        self.request(|reply| Command::ListIntents { reply })
    }

    pub fn events(&self, intent_id: &IntentId) -> Result<Vec<StoredEvent>> {
        self.request(|reply| Command::Events {
            intent_id: intent_id.clone(),
            reply,
        })
    }

    pub fn capabilities(&self, intent_id: &IntentId) -> Result<Vec<CapabilityRegistration>> {
        self.request(|reply| Command::Capabilities {
            intent_id: intent_id.clone(),
            reply,
        })
    }

    pub fn agent_bindings(&self, intent_id: &IntentId) -> Result<Vec<AgentBinding>> {
        self.request(|reply| Command::Bindings {
            intent_id: intent_id.clone(),
            reply,
        })
    }

    pub fn register_capability(
        &self,
        intent_id: &IntentId,
        capability: CapabilityRegistration,
    ) -> Result<()> {
        self.request(|reply| Command::RegisterCapability {
            intent_id: intent_id.clone(),
            capability,
            reply,
        })
    }

    pub fn discover_mcp_tools(
        &self,
        intent_id: &IntentId,
        server_name: &str,
        command: &str,
    ) -> Result<Vec<CapabilityRegistration>> {
        self.request(|reply| Command::DiscoverMcp {
            intent_id: intent_id.clone(),
            server_name: server_name.into(),
            command: command.into(),
            reply,
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn bind_agent(
        &self,
        intent_id: &IntentId,
        agent_id: AgentId,
        name: &str,
        scope_id: ScopeId,
        run_id: Option<RunId>,
        outbound: AgentTransport,
        capability_ids: Vec<CapabilityId>,
        ttl_seconds: i64,
    ) -> Result<AgentBinding> {
        self.request(|reply| Command::BindAgent {
            intent_id: intent_id.clone(),
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
        intent_id: &IntentId,
        binding_id: &AgentBindingId,
        reason: &str,
    ) -> Result<()> {
        self.request(|reply| Command::RevokeBinding {
            intent_id: intent_id.clone(),
            binding_id: binding_id.clone(),
            reason: reason.into(),
            reply,
        })
    }

    pub fn dispatch_agent_binding(
        &self,
        intent_id: &IntentId,
        binding_id: &AgentBindingId,
    ) -> Result<IntentProjection> {
        self.request(|reply| Command::DispatchBinding {
            intent_id: intent_id.clone(),
            binding_id: binding_id.clone(),
            reply,
        })
    }

    pub fn record_agent_binding_event(
        &self,
        intent_id: &IntentId,
        binding_id: &AgentBindingId,
        session: &str,
        event: AgentBindingEvent,
    ) -> Result<IntentProjection> {
        self.request(|reply| Command::RecordBindingEvent {
            intent_id: intent_id.clone(),
            binding_id: binding_id.clone(),
            session: session.into(),
            event,
            reply,
        })
    }

    pub fn transition_agent_binding(
        &self,
        intent_id: &IntentId,
        binding_id: &AgentBindingId,
        state: AgentBindingState,
    ) -> Result<()> {
        self.request(|reply| Command::TransitionBinding {
            intent_id: intent_id.clone(),
            binding_id: binding_id.clone(),
            state,
            reply,
        })
    }

    pub fn register_calendar_connector(
        &self,
        intent_id: &IntentId,
        name: &str,
        source: &str,
    ) -> Result<ConnectorRegistration> {
        self.request(|reply| Command::RegisterCalendar {
            intent_id: intent_id.clone(),
            name: name.into(),
            source: source.into(),
            reply,
        })
    }

    pub fn register_linear_connector(
        &self,
        intent_id: &IntentId,
        name: &str,
        source: &str,
    ) -> Result<ConnectorRegistration> {
        self.request(|reply| Command::RegisterLinear {
            intent_id: intent_id.clone(),
            name: name.into(),
            source: source.into(),
            reply,
        })
    }

    pub fn register_connector_adapter(
        &self,
        intent_id: &IntentId,
        name: &str,
        source: &str,
        adapter: &str,
    ) -> Result<ConnectorRegistration> {
        self.register_connector_adapter_with_credential(intent_id, name, source, adapter, None, 300)
    }

    pub fn register_connector_adapter_with_credential(
        &self,
        intent_id: &IntentId,
        name: &str,
        source: &str,
        adapter: &str,
        credential: Option<ConnectorCredential>,
        sync_interval_seconds: u64,
    ) -> Result<ConnectorRegistration> {
        self.request(|reply| Command::RegisterConnectorAdapter {
            intent_id: intent_id.clone(),
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
        intent_id: &IntentId,
        connector_id: &ConnectorId,
        credential: ConnectorCredential,
    ) -> Result<ConnectorCredentialMetadata> {
        self.request(|reply| Command::AttachConnectorCredential {
            intent_id: intent_id.clone(),
            connector_id: connector_id.clone(),
            credential,
            reply,
        })
    }

    pub fn revoke_connector_credential(
        &self,
        intent_id: &IntentId,
        connector_id: &ConnectorId,
    ) -> Result<ConnectorCredentialMetadata> {
        self.request(|reply| Command::RevokeConnectorCredential {
            intent_id: intent_id.clone(),
            connector_id: connector_id.clone(),
            reply,
        })
    }

    pub fn connector_health(
        &self,
        intent_id: &IntentId,
        connector_id: &ConnectorId,
    ) -> Result<ConnectorHealthReport> {
        self.request(|reply| Command::ConnectorHealth {
            intent_id: intent_id.clone(),
            connector_id: connector_id.clone(),
            reply,
        })
    }

    pub fn propose_connector_action(
        &self,
        intent_id: &IntentId,
        connector_id: &ConnectorId,
        capability: &str,
        target: &str,
        input: serde_json::Value,
        idempotency_key: Option<String>,
    ) -> Result<ConnectorAction> {
        self.request(|reply| Command::ProposeConnectorAction {
            intent_id: intent_id.clone(),
            connector_id: connector_id.clone(),
            capability: capability.into(),
            target: target.into(),
            input,
            idempotency_key,
            reply,
        })
    }

    pub fn execute_connector_action(
        &self,
        intent_id: &IntentId,
        action_id: &ConnectorActionId,
    ) -> Result<ConnectorAction> {
        self.request(|reply| Command::ExecuteConnectorAction {
            intent_id: intent_id.clone(),
            action_id: action_id.clone(),
            reply,
        })
    }

    pub fn connector_actions(&self, intent_id: &IntentId) -> Result<Vec<ConnectorAction>> {
        self.request(|reply| Command::ConnectorActions {
            intent_id: intent_id.clone(),
            reply,
        })
    }

    pub fn verify_connector_action(
        &self,
        intent_id: &IntentId,
        action_id: &ConnectorActionId,
    ) -> Result<ConnectorAction> {
        self.request(|reply| Command::VerifyConnectorAction {
            intent_id: intent_id.clone(),
            action_id: action_id.clone(),
            reply,
        })
    }

    pub fn compensate_connector_action(
        &self,
        intent_id: &IntentId,
        action_id: &ConnectorActionId,
    ) -> Result<ConnectorAction> {
        self.request(|reply| Command::CompensateConnectorAction {
            intent_id: intent_id.clone(),
            action_id: action_id.clone(),
            reply,
        })
    }

    pub fn connector_manifests(&self) -> Result<Vec<ConnectorManifest>> {
        self.request(|reply| Command::ConnectorAdapters { reply })
    }

    pub fn sync_connector(
        &self,
        intent_id: &IntentId,
        connector_id: &ConnectorId,
    ) -> Result<ConnectorSyncResult> {
        self.request(|reply| Command::SyncConnector {
            intent_id: intent_id.clone(),
            connector_id: connector_id.clone(),
            reply,
        })
    }

    pub fn ingest_connector_webhook(
        &self,
        intent_id: &IntentId,
        connector_id: &ConnectorId,
        session_token: &str,
        payload: serde_json::Value,
    ) -> Result<ConnectorSyncResult> {
        self.request(|reply| Command::IngestConnectorWebhook {
            intent_id: intent_id.clone(),
            connector_id: connector_id.clone(),
            session_token: session_token.into(),
            payload,
            reply,
        })
    }

    pub fn configure_connector_policy(
        &self,
        intent_id: &IntentId,
        policy: ConnectorPolicy,
    ) -> Result<IntentSpec> {
        self.request(|reply| Command::ConfigureConnectorPolicy {
            intent_id: intent_id.clone(),
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

    pub fn intent_candidates(
        &self,
        cursor: u64,
    ) -> Result<Vec<else_protocol::connector::IntentCandidateRecord>> {
        self.request(|reply| Command::IntentCandidates { cursor, reply })
    }

    pub fn world_items(
        &self,
        intent_id: &IntentId,
        include_resolved: bool,
    ) -> Result<Vec<else_protocol::world::WorldItem>> {
        self.request(|reply| Command::WorldItems {
            intent_id: intent_id.clone(),
            include_resolved,
            reply,
        })
    }

    pub fn personal_brief(&self) -> Result<else_protocol::world::PersonalBrief> {
        self.request(|reply| Command::PersonalBrief { reply })
    }

    pub fn resolve_world_item(
        &self,
        intent_id: &IntentId,
        item_id: &else_protocol::ids::WorldItemId,
        kind: else_protocol::world::WorldItemResolutionKind,
        note: Option<String>,
    ) -> Result<else_protocol::world::WorldItem> {
        self.request(|reply| Command::ResolveWorldItem {
            intent_id: intent_id.clone(),
            item_id: item_id.clone(),
            kind,
            note,
            reply,
        })
    }

    pub fn project_worlds(
        &self,
        intent_id: Option<&IntentId>,
        include_resolved: bool,
    ) -> Result<Vec<else_protocol::world::ProjectWorld>> {
        self.request(|reply| Command::ProjectWorlds {
            intent_id: intent_id.cloned(),
            include_resolved,
            reply,
        })
    }

    pub fn resolve_signals(&self, intent_id: &IntentId) -> Result<Vec<IntentCandidate>> {
        self.request(|reply| Command::ResolveSignals {
            intent_id: intent_id.clone(),
            reply,
        })
    }

    pub fn propose_intent_candidate(
        &self,
        intent_id: &IntentId,
        candidate: IntentCandidate,
    ) -> Result<IntentCandidate> {
        self.request(|reply| Command::ProposeCandidate {
            intent_id: intent_id.clone(),
            candidate,
            reply,
        })
    }

    pub fn resolve_intent_candidate(
        &self,
        intent_id: &IntentId,
        candidate_id: &IntentCandidateId,
        accept: bool,
    ) -> Result<Option<IntentSpec>> {
        self.request(|reply| Command::ResolveCandidate {
            intent_id: intent_id.clone(),
            candidate_id: candidate_id.clone(),
            accept,
            reply,
        })
    }

    pub fn record_external_event(
        &self,
        intent_id: &IntentId,
        source: &str,
        summary: &str,
    ) -> Result<IntentProjection> {
        self.request(|reply| Command::RecordExternalEvent {
            intent_id: intent_id.clone(),
            source: source.to_string(),
            summary: summary.to_string(),
            reply,
        })
    }

    pub fn wait_for_external(
        &self,
        intent_id: &IntentId,
        reason: &str,
    ) -> Result<IntentProjection> {
        self.request(|reply| Command::WaitForExternal {
            intent_id: intent_id.clone(),
            reason: reason.to_string(),
            reply,
        })
    }

    pub fn schedule_timer(
        &self,
        intent_id: &IntentId,
        fire_at: DateTime<Utc>,
        reason: &str,
    ) -> Result<TimerId> {
        self.request(|reply| Command::ScheduleTimer {
            intent_id: intent_id.clone(),
            fire_at,
            reason: reason.to_string(),
            reply,
        })
    }

    pub fn decide(
        &self,
        intent_id: &IntentId,
        request_id: &AttentionId,
        choice: DecisionChoice,
    ) -> Result<IntentProjection> {
        self.request(|reply| Command::Decide {
            intent_id: intent_id.clone(),
            request_id: request_id.clone(),
            choice,
            reply,
        })
    }

    pub fn submit_attention_request(
        &self,
        intent_id: &IntentId,
        scope_id: Option<ScopeId>,
        request: AttentionRequest,
    ) -> Result<AttentionId> {
        self.request(|reply| Command::SubmitAttention {
            intent_id: intent_id.clone(),
            scope_id,
            request,
            reply,
        })
    }

    pub fn propose_plan(&self, intent_id: &IntentId, plan: Plan) -> Result<Plan> {
        self.request(|reply| Command::ProposePlan {
            intent_id: intent_id.clone(),
            plan,
            reply,
        })
    }

    pub fn abort(&self, intent_id: &IntentId, reason: &str) -> Result<IntentProjection> {
        self.request(|reply| Command::Abort {
            intent_id: intent_id.clone(),
            reason: reason.to_string(),
            reply,
        })
    }

    pub fn drive(
        &self,
        intent_id: &IntentId,
        harness_hint: Option<&str>,
        max_steps: u64,
    ) -> Result<KernelDrive> {
        self.request(|reply| Command::Drive {
            intent_id: intent_id.clone(),
            harness_hint: harness_hint.map(str::to_string),
            max_steps,
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
        Command::CreateIntent {
            outcome,
            project,
            reply,
        } => {
            let _ = reply.send(runtime.create_intent(&outcome, project));
        }
        Command::Projection { intent_id, reply } => {
            let _ = reply.send(runtime.projection(&intent_id));
        }
        Command::ListIntents { reply } => {
            let _ = reply.send(runtime.list_intents());
        }
        Command::Events { intent_id, reply } => {
            let _ = reply.send(runtime.events(&intent_id));
        }
        Command::Capabilities { intent_id, reply } => {
            let _ = reply.send(runtime.capabilities(&intent_id));
        }
        Command::Bindings { intent_id, reply } => {
            let _ = reply.send(runtime.agent_bindings(&intent_id));
        }
        Command::RegisterCapability {
            intent_id,
            capability,
            reply,
        } => {
            let _ = reply.send(runtime.register_capability(&intent_id, capability));
        }
        Command::DiscoverMcp {
            intent_id,
            server_name,
            command,
            reply,
        } => {
            let _ = reply.send(runtime.discover_mcp_tools(&intent_id, &server_name, &command));
        }
        Command::BindAgent {
            intent_id,
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
                &intent_id,
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
            intent_id,
            binding_id,
            reason,
            reply,
        } => {
            let _ = reply.send(runtime.revoke_agent_binding(&intent_id, &binding_id, &reason));
        }
        Command::DispatchBinding {
            intent_id,
            binding_id,
            reply,
        } => {
            let _ = reply.send(runtime.dispatch_agent_binding(&intent_id, &binding_id));
        }
        Command::RecordBindingEvent {
            intent_id,
            binding_id,
            session,
            event,
            reply,
        } => {
            let _ = reply.send(runtime.record_agent_binding_event(
                &intent_id,
                &binding_id,
                &session,
                event,
            ));
        }
        Command::TransitionBinding {
            intent_id,
            binding_id,
            state,
            reply,
        } => {
            let _ = reply.send(runtime.transition_agent_binding(&intent_id, &binding_id, state));
        }
        Command::RegisterCalendar {
            intent_id,
            name,
            source,
            reply,
        } => {
            let _ = reply.send(runtime.register_calendar_connector(&intent_id, &name, &source));
        }
        Command::RegisterLinear {
            intent_id,
            name,
            source,
            reply,
        } => {
            let _ = reply.send(runtime.register_linear_connector(&intent_id, &name, &source));
        }
        Command::RegisterConnectorAdapter {
            intent_id,
            name,
            source,
            adapter,
            credential,
            sync_interval_seconds,
            reply,
        } => {
            let _ = reply.send(runtime.register_connector_adapter_with_credential(
                &intent_id,
                &name,
                &source,
                &adapter,
                credential,
                sync_interval_seconds,
            ));
        }
        Command::AttachConnectorCredential {
            intent_id,
            connector_id,
            credential,
            reply,
        } => {
            let _ = reply.send(runtime.attach_connector_credential(
                &intent_id,
                &connector_id,
                credential,
            ));
        }
        Command::RevokeConnectorCredential {
            intent_id,
            connector_id,
            reply,
        } => {
            let _ = reply.send(runtime.revoke_connector_credential(&intent_id, &connector_id));
        }
        Command::ConnectorHealth {
            intent_id,
            connector_id,
            reply,
        } => {
            let _ = reply.send(runtime.connector_health(&intent_id, &connector_id));
        }
        Command::ProposeConnectorAction {
            intent_id,
            connector_id,
            capability,
            target,
            input,
            idempotency_key,
            reply,
        } => {
            let _ = reply.send(runtime.propose_connector_action(
                &intent_id,
                &connector_id,
                &capability,
                &target,
                input,
                idempotency_key,
            ));
        }
        Command::ExecuteConnectorAction {
            intent_id,
            action_id,
            reply,
        } => {
            let _ = reply.send(runtime.execute_connector_action(&intent_id, &action_id));
        }
        Command::VerifyConnectorAction {
            intent_id,
            action_id,
            reply,
        } => {
            let _ = reply.send(runtime.verify_connector_action(&intent_id, &action_id));
        }
        Command::CompensateConnectorAction {
            intent_id,
            action_id,
            reply,
        } => {
            let _ = reply.send(runtime.compensate_connector_action(&intent_id, &action_id));
        }
        Command::ConnectorActions { intent_id, reply } => {
            let _ = reply.send(runtime.connector_actions(&intent_id));
        }
        Command::ConnectorAdapters { reply } => {
            let _ = reply.send(Ok(runtime.connector_manifests()));
        }
        Command::SyncConnector {
            intent_id,
            connector_id,
            reply,
        } => {
            let _ = reply.send(runtime.sync_connector(&intent_id, &connector_id));
        }
        Command::IngestConnectorWebhook {
            intent_id,
            connector_id,
            session_token,
            payload,
            reply,
        } => {
            let _ = reply.send(runtime.ingest_connector_webhook(
                &intent_id,
                &connector_id,
                &session_token,
                payload,
            ));
        }
        Command::ConfigureConnectorPolicy {
            intent_id,
            policy,
            reply,
        } => {
            let _ = reply.send(runtime.configure_connector_policy(&intent_id, policy));
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
        Command::IntentCandidates { cursor, reply } => {
            let _ = reply.send(runtime.intent_candidates(cursor));
        }
        Command::WorldItems {
            intent_id,
            include_resolved,
            reply,
        } => {
            let _ = reply.send(runtime.world_items(&intent_id, include_resolved));
        }
        Command::PersonalBrief { reply } => {
            let _ = reply.send(runtime.personal_brief());
        }
        Command::ResolveWorldItem {
            intent_id,
            item_id,
            kind,
            note,
            reply,
        } => {
            let _ = reply.send(runtime.resolve_world_item(&intent_id, &item_id, kind, note));
        }
        Command::ProjectWorlds {
            intent_id,
            include_resolved,
            reply,
        } => {
            let _ = reply.send(runtime.project_worlds(intent_id.as_ref(), include_resolved));
        }
        Command::ResolveSignals { intent_id, reply } => {
            let _ = reply.send(runtime.resolve_signals(&intent_id));
        }
        Command::ProposeCandidate {
            intent_id,
            candidate,
            reply,
        } => {
            let _ = reply.send(runtime.propose_intent_candidate(&intent_id, candidate));
        }
        Command::ResolveCandidate {
            intent_id,
            candidate_id,
            accept,
            reply,
        } => {
            let _ = reply.send(runtime.resolve_intent_candidate(&intent_id, &candidate_id, accept));
        }
        Command::RecordExternalEvent {
            intent_id,
            source,
            summary,
            reply,
        } => {
            let _ = reply.send(runtime.record_external_event(&intent_id, &source, &summary));
        }
        Command::WaitForExternal {
            intent_id,
            reason,
            reply,
        } => {
            let _ = reply.send(runtime.wait_for_external(&intent_id, &reason));
        }
        Command::ScheduleTimer {
            intent_id,
            fire_at,
            reason,
            reply,
        } => {
            let _ = reply.send(runtime.schedule_timer(&intent_id, fire_at, &reason));
        }
        Command::Decide {
            intent_id,
            request_id,
            choice,
            reply,
        } => {
            let _ = reply.send(runtime.decide(&intent_id, &request_id, choice));
        }
        Command::SubmitAttention {
            intent_id,
            scope_id,
            request,
            reply,
        } => {
            let result = match scope_id {
                Some(scope_id) => {
                    runtime.submit_attention_request_for_scope(&intent_id, &scope_id, &request)
                }
                None => runtime.submit_attention_request(&intent_id, &request),
            };
            let _ = reply.send(result);
        }
        Command::ProposePlan {
            intent_id,
            plan,
            reply,
        } => {
            let _ = reply.send(runtime.propose_plan(&intent_id, plan));
        }
        Command::Abort {
            intent_id,
            reason,
            reply,
        } => {
            let _ = reply.send(runtime.abort(&intent_id, &reason));
        }
        Command::Drive {
            intent_id,
            harness_hint,
            max_steps,
            reply,
        } => {
            let _ = reply.send(runtime.drive(&intent_id, harness_hint.as_deref(), max_steps));
        }
        Command::Shutdown { reply } => {
            let _ = reply.send(Ok(()));
        }
    }
}

fn reconcile_all(runtime: &Runtime) -> Option<DateTime<Utc>> {
    let _ = runtime.refresh_dirty_mcp_servers();
    let mut next_wake = runtime.sync_due_connectors(Utc::now());
    let Ok(intents) = runtime.list_intents() else {
        return Some(Utc::now() + chrono::Duration::seconds(2));
    };
    let mut had_error = false;
    for spec in intents {
        let Ok(projection) = runtime.projection(&spec.id) else {
            had_error = true;
            continue;
        };
        if matches!(
            projection.state,
            Some(
                else_protocol::intent::IntentState::Completed
                    | else_protocol::intent::IntentState::Failed
                    | else_protocol::intent::IntentState::Cancelled
            )
        ) {
            continue;
        }
        if runtime.drive(&spec.id, None, 16).is_err() {
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
