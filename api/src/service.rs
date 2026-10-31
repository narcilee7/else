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
use else_protocol::ids::{
    AgentBindingId, AgentId, AttentionId, CapabilityId, IntentId, RunId, ScopeId, TimerId,
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
    let Ok(intents) = runtime.list_intents() else {
        return Some(Utc::now() + chrono::Duration::seconds(2));
    };
    let mut next_wake = None;
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
