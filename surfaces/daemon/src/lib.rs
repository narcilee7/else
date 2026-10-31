//! Loopback HTTP surface for the resident Intent Runtime.

use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;
use std::time::Instant;

use chrono::Utc;
use else_api::{ApiError, RuntimeHandle, RuntimeService};
use else_ledger::{IntentProjection, StoredEvent};
use else_protocol::api::{AttentionWindowSnapshot, EventResponse, IntentSnapshot};
use else_protocol::attention::DecisionChoice;
use else_protocol::intent::{IntentSpec, IntentState};
use fs2::FileExt;
use serde::Serialize;
use serde_json::{json, Value};
use tiny_http::{Header, Method, Request, Response, Server, StatusCode};

#[derive(Debug, thiserror::Error)]
pub enum DaemonError {
    #[error("runtime: {0}")]
    Api(#[from] ApiError),
    #[error("http: {0}")]
    Http(#[from] std::io::Error),
    #[error("bind: {0}")]
    Bind(String),
    #[error("runtime already active: {0}")]
    AlreadyRunning(String),
    #[error("invalid request: {0}")]
    InvalidRequest(String),
}

pub type Result<T> = std::result::Result<T, DaemonError>;

pub struct DaemonConfig {
    pub dir: PathBuf,
    pub bind: String,
    pub token: Option<String>,
}

pub struct Daemon {
    pub url: String,
    pub token: String,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
    _lock: File,
}

impl Daemon {
    pub fn start(config: DaemonConfig) -> Result<Self> {
        std::fs::create_dir_all(&config.dir)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&config.dir, std::fs::Permissions::from_mode(0o700))?;
        }
        let lock_path = config.dir.join("daemon.lock");
        let lock = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(&lock_path)?;
        lock.try_lock_exclusive()
            .map_err(|_| DaemonError::AlreadyRunning(lock_path.display().to_string()))?;
        let token = config
            .token
            .or_else(|| std::env::var("ELSE_RUNTIME_TOKEN").ok())
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

        // 先监听，再开账本，最后写令牌。
        //
        // 顺序很重要：绑定失败时不能已经把令牌写进 runtime 目录 ——
        // 否则那份令牌对谁都不成立，却会让下一次连接拿着错令牌来敲门。
        let server =
            Server::http(&config.bind).map_err(|error| DaemonError::Bind(error.to_string()))?;
        let addr = server
            .server_addr()
            .to_ip()
            .ok_or_else(|| DaemonError::Bind("daemon requires a TCP listener".into()))?;
        let url = format!("http://{addr}");

        let service = RuntimeService::start(&config.dir)?;
        let handle = service.handle();
        write_token(&config.dir, &token)?;

        let state = Arc::new(AppState {
            token: token.clone(),
            handle: handle.clone(),
            operations: Arc::new(Mutex::new(HashMap::new())),
            idempotency: Arc::new(Mutex::new(HashMap::new())),
        });
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = stop.clone();
        let worker = thread::spawn(move || {
            while !worker_stop.load(Ordering::SeqCst) {
                while let Ok(Some(request)) = server.recv_timeout(Duration::from_millis(100)) {
                    let state = state.clone();
                    thread::spawn(move || handle_http(request, &state));
                }
            }
            service.shutdown();
        });
        Ok(Self {
            url,
            token,
            stop,
            worker: Some(worker),
            _lock: lock,
        })
    }

    pub fn shutdown(mut self) {
        self.stop();
    }

    fn stop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

impl Drop for Daemon {
    fn drop(&mut self) {
        self.stop();
    }
}

struct AppState {
    token: String,
    handle: RuntimeHandle,
    operations: Arc<Mutex<HashMap<String, Operation>>>,
    idempotency: Arc<Mutex<HashMap<String, CachedResponse>>>,
}

#[derive(Clone)]
struct CachedResponse {
    status: u16,
    body: String,
}

#[derive(Clone, Serialize)]
struct Operation {
    id: String,
    intent_id: String,
    kind: String,
    status: String,
    error: Option<String>,
    /// 操作产出的结果。drive 会带上停止原因与步数。
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
}

fn handle_http(mut request: Request, state: &AppState) {
    let path = request.url().split('?').next().unwrap_or("/").to_string();
    if path == "/v1/health" {
        respond_json(request, 200, &json!({ "ok": true, "version": "0.1.0" }));
        return;
    }
    if path == "/v1/binding-events" {
        let mut body = String::new();
        if request.as_reader().read_to_string(&mut body).is_err() {
            respond_json(request, 400, &json!({"error":"invalid body"}));
            return;
        }
        let token = header(&request, "Authorization")
            .and_then(|value| value.strip_prefix("Bearer ").map(str::to_string))
            .unwrap_or_default();
        let value: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
        let intent_id =
            else_protocol::ids::IntentId::from(value["intent_id"].as_str().unwrap_or_default());
        let binding_id = else_protocol::ids::AgentBindingId::from(
            value["binding_id"].as_str().unwrap_or_default(),
        );
        let event: else_protocol::capability::AgentBindingEvent =
            match serde_json::from_value(value["event"].clone()) {
                Ok(event) => event,
                Err(error) => {
                    respond_json(request, 400, &json!({"error":error.to_string()}));
                    return;
                }
            };
        match state
            .handle
            .record_agent_binding_event(&intent_id, &binding_id, &token, event)
        {
            Ok(projection) => {
                respond_json(request, 200, &json!({"ok":true,"cursor":projection.cursor}));
            }
            Err(error) => {
                respond_json(request, 403, &json!({"error":error.to_string()}));
            }
        }
        return;
    }
    if path == "/v1/connector-events" {
        let mut body = String::new();
        if request.as_reader().read_to_string(&mut body).is_err() {
            respond_json(request, 400, &json!({"error":"invalid body"}));
            return;
        }
        let token = header(&request, "Authorization")
            .and_then(|value| value.strip_prefix("Bearer ").map(str::to_string))
            .unwrap_or_default();
        let value: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
        let intent_id =
            else_protocol::ids::IntentId::from(value["intent_id"].as_str().unwrap_or_default());
        let connector_id = else_protocol::ids::ConnectorId::from(
            value["connector_id"].as_str().unwrap_or_default(),
        );
        match state.handle.ingest_connector_webhook(
            &intent_id,
            &connector_id,
            &token,
            value["payload"].clone(),
        ) {
            Ok(result) => {
                respond_json(request, 200, &json!({"ok":true,"result":result}));
            }
            Err(ApiError::Unauthorized(error)) => {
                respond_json(request, 403, &json!({"error":error}));
            }
            Err(error) => {
                respond_json(request, 400, &json!({"error":error.to_string()}));
            }
        }
        return;
    }
    if !authorized(&request, &state.token) {
        respond_json(request, 401, &json!({ "error": "unauthorized" }));
        return;
    }
    let mut body = String::new();
    if request.method() != &Method::Get && request.as_reader().read_to_string(&mut body).is_err() {
        respond_json(request, 400, &json!({ "error": "invalid body" }));
        return;
    }
    let path_segments: Vec<_> = path.trim_matches('/').split('/').collect();
    if path_segments.len() == 7
        && path_segments[0] == "v1"
        && path_segments[1] == "intents"
        && path_segments[3] == "bindings"
        && path_segments[5] == "events"
        && path_segments[6] == "stream"
        && request.method() == &Method::Get
    {
        let intent_id = else_protocol::ids::IntentId::from(path_segments[2]);
        let binding_id = else_protocol::ids::AgentBindingId::from(path_segments[4]);
        let cursor = query_value(request.url(), "cursor")
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(0);
        let reader = BindingEventStream::new(state.handle.clone(), intent_id, binding_id, cursor);
        let response = Response::new(
            StatusCode(200),
            vec![
                Header::from_bytes("Content-Type", "text/event-stream").expect("static header"),
                Header::from_bytes("Cache-Control", "no-cache").expect("static header"),
            ],
            reader,
            None,
            None,
        );
        let _ = request.respond(response);
        return;
    }
    if path_segments.len() == 3
        && path_segments[0] == "v1"
        && path_segments[1] == "events"
        && path_segments[2] == "stream"
        && request.method() == &Method::Get
    {
        let cursor = query_value(request.url(), "cursor")
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(0);
        let reader = EventStream::new(state.handle.clone(), cursor);
        let response = Response::new(
            StatusCode(200),
            vec![
                Header::from_bytes("Content-Type", "text/event-stream").expect("static header"),
                Header::from_bytes("Cache-Control", "no-cache").expect("static header"),
            ],
            reader,
            None,
            None,
        );
        let _ = request.respond(response);
        return;
    }
    let idempotency_key = header(&request, "Idempotency-Key");
    if request.method() != &Method::Get && idempotency_key.is_none() {
        respond_json(
            request,
            400,
            &json!({ "error": "Idempotency-Key header is required" }),
        );
        return;
    }
    if request.method() != &Method::Get {
        let key = idempotency_key.clone().unwrap();
        if let Some(cached) = state
            .idempotency
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .get(&key)
            .cloned()
        {
            respond(request, cached.status, &cached.body);
            return;
        }
    }

    let result = route(&request, state, &path_segments, &body);
    match result {
        Ok((status, response_body)) => {
            if let Some(key) = idempotency_key {
                state
                    .idempotency
                    .lock()
                    .unwrap_or_else(|poison| poison.into_inner())
                    .insert(
                        key,
                        CachedResponse {
                            status,
                            body: response_body.clone(),
                        },
                    );
            }
            respond(request, status, &response_body);
        }
        Err(error) => {
            let status = match error {
                DaemonError::Api(ApiError::IntentNotFound(_))
                | DaemonError::Api(ApiError::NotFound(_)) => 404,
                DaemonError::Api(ApiError::Conflict(_))
                | DaemonError::Api(ApiError::Gateway(_)) => 409,
                DaemonError::Api(ApiError::Unauthorized(_)) => 403,
                DaemonError::InvalidRequest(_) => 400,
                _ => 500,
            };
            respond_json(request, status, &json!({ "error": error.to_string() }));
        }
    }
}

fn route(
    request: &Request,
    state: &AppState,
    segments: &[&str],
    body: &str,
) -> Result<(u16, String)> {
    if segments == ["v1", "version"] && request.method() == &Method::Get {
        // 鉴权端点：客户端用它确认「连上的这个 Runtime 用的是这份契约」。
        return Ok((
            200,
            json_string(&json!({
                "contract_version": else_protocol::api::CONTRACT_VERSION,
                "protocol_version": else_protocol::PROTOCOL_VERSION,
            }))?,
        ));
    }
    if segments == ["v1", "export"] && request.method() == &Method::Get {
        return Ok((200, json_string(&state.handle.export_data()?)?));
    }
    if segments == ["v1", "diagnostics"] && request.method() == &Method::Get {
        return Ok((200, json_string(&state.handle.diagnostics()?)?));
    }
    if segments == ["v1", "diagnostics", "leases", "cleanup"] && request.method() == &Method::Post {
        let report = state.handle.cleanup_orphan_leases()?;
        return Ok((200, json_string(&report)?));
    }
    if segments == ["v1", "settings"] {
        match request.method() {
            Method::Get => {
                return Ok((200, json_string(&state.handle.runtime_settings()?)?));
            }
            Method::Post => {
                let settings: else_api::RuntimeSettings =
                    serde_json::from_str(body).map_err(json_error)?;
                let settings = state.handle.update_runtime_settings(settings)?;
                return Ok((200, json_string(&settings)?));
            }
            _ => {}
        }
    }
    if segments == ["v1", "intents"] {
        return match request.method() {
            Method::Get => {
                let specs = state.handle.list_intents()?;
                let mut snapshots = Vec::new();
                for spec in specs {
                    snapshots.push(snapshot(&state.handle.projection(&spec.id)?));
                }
                Ok((200, serde_json::to_string(&snapshots).map_err(json_error)?))
            }
            Method::Post => {
                let value: Value = serde_json::from_str(body).map_err(json_error)?;
                let outcome = value["outcome"]
                    .as_str()
                    .ok_or_else(|| DaemonError::InvalidRequest("missing outcome".into()))?;
                let project = value["project"].as_str().map(str::to_string);
                let spec = state.handle.create_intent(outcome, project)?;
                let projection = state.handle.projection(&spec.id)?;
                Ok((200, json_string(&snapshot(&projection))?))
            }
            _ => Ok((405, json!({ "error": "method not allowed" }).to_string())),
        };
    }
    if segments == ["v1", "signals"] && request.method() == &Method::Get {
        let cursor = query_value(request.url(), "cursor")
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(0);
        let provider = query_value(request.url(), "provider");
        let entity = query_value(request.url(), "entity");
        let mut signals =
            state
                .handle
                .world_signals(cursor, provider.as_deref(), entity.as_deref())?;
        signals.sort_by_key(|record| record.seq);
        return Ok((200, json_string(&signals)?));
    }
    if segments == ["v1", "candidates"] && request.method() == &Method::Get {
        let cursor = query_value(request.url(), "cursor")
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(0);
        let mut candidates = state.handle.intent_candidates(cursor)?;
        candidates.sort_by_key(|record| record.seq);
        return Ok((200, json_string(&candidates)?));
    }
    if segments == ["v1", "connector-adapters"] && request.method() == &Method::Get {
        let manifests = state.handle.connector_manifests()?;
        return Ok((200, json_string(&manifests)?));
    }
    if segments == ["v1", "harnesses"] && request.method() == &Method::Get {
        let views: Vec<_> = else_harness::builtin_adapters()
            .iter()
            .map(|adapter| else_protocol::api::HarnessView {
                name: adapter.name().to_string(),
                capabilities: adapter.capabilities(),
                network_access: adapter.network_access(),
            })
            .collect();
        return Ok((200, json_string(&views)?));
    }
    if segments == ["v1", "world", "brief"] && request.method() == &Method::Get {
        let brief = state.handle.personal_brief()?;
        return Ok((200, json_string(&brief)?));
    }
    if segments == ["v1", "attention"] && request.method() == &Method::Get {
        let now = Utc::now();
        let mut windows = Vec::new();
        for spec in state.handle.list_intents()? {
            let projection = state.handle.projection(&spec.id)?;
            for (window, requests) in projection.actionable_attention_windows(now) {
                windows.push(AttentionWindowSnapshot {
                    intent_id: spec.id.clone(),
                    window: window.clone(),
                    requests: requests.into_iter().cloned().collect(),
                });
            }
        }
        windows.sort_by_key(|window| (window.window.due_at, window.intent_id.clone()));
        return Ok((200, json_string(&windows)?));
    }
    if segments == ["v1", "projects"] && request.method() == &Method::Get {
        let include_resolved = query_value(request.url(), "include_resolved")
            .is_some_and(|value| matches!(value.as_str(), "1" | "true" | "yes"));
        let projects = state.handle.project_worlds(None, include_resolved)?;
        return Ok((200, json_string(&projects)?));
    }
    if segments == ["v1", "identities"] && request.method() == &Method::Get {
        let include_resolved = query_value(request.url(), "include_resolved")
            .is_some_and(|value| matches!(value.as_str(), "1" | "true" | "yes"));
        let intent_id =
            query_value(request.url(), "intent_id").map(else_protocol::ids::IntentId::from);
        let identities = state
            .handle
            .identity_worlds(intent_id.as_ref(), include_resolved)?;
        return Ok((200, json_string(&identities)?));
    }
    if segments.len() >= 3 && segments[0..2] == ["v1", "intents"] {
        let intent_id = else_protocol::ids::IntentId::from(segments[2]);
        if segments.len() == 3 {
            match request.method() {
                Method::Get => {
                    return Ok((
                        200,
                        json_string(&snapshot(&state.handle.projection(&intent_id)?))?,
                    ))
                }
                Method::Delete => {
                    check_expected_revision(request, &state.handle.projection(&intent_id)?)?;
                    let report = state.handle.delete_intent(&intent_id)?;
                    return Ok((200, json_string(&report)?));
                }
                _ => {}
            }
        }
        if segments.len() == 5
            && segments[3] == "repair"
            && segments[4] == "retry"
            && request.method() == &Method::Post
        {
            check_expected_revision(request, &state.handle.projection(&intent_id)?)?;
            let (retried_scopes, projection) = state.handle.retry_failed_scopes(&intent_id)?;
            return Ok((
                200,
                json_string(&json!({
                    "retried_scopes": retried_scopes,
                    "intent": snapshot(&projection),
                }))?,
            ));
        }
        if segments.len() == 4 && segments[3] == "run" && request.method() == &Method::Post {
            check_expected_revision(request, &state.handle.projection(&intent_id)?)?;
            let value: Value = serde_json::from_str(body).unwrap_or_else(|_| json!({}));
            let harness = value["harness"].as_str().map(str::to_string);
            let mut current = snapshot(&state.handle.projection(&intent_id)?);
            current.operation_id = Some(spawn_operation(state, "run", &intent_id, {
                let handle = state.handle.clone();
                let intent_id = intent_id.clone();
                move || {
                    handle
                        .drive(&intent_id, harness.as_deref(), 16)
                        .map(|driven| {
                            json!({
                                "stopped": format!("{:?}", driven.stopped),
                                "steps": driven.steps,
                            })
                        })
                }
            }));
            return Ok((202, json_string(&current)?));
        }
        if segments.len() == 4 && segments[3] == "capabilities" {
            match request.method() {
                Method::Get => {
                    return Ok((200, json_string(&state.handle.capabilities(&intent_id)?)?));
                }
                Method::Post => {
                    let capability: else_protocol::capability::CapabilityRegistration =
                        serde_json::from_str(body).map_err(json_error)?;
                    state
                        .handle
                        .register_capability(&intent_id, capability.clone())?;
                    return Ok((200, json_string(&capability)?));
                }
                _ => {}
            }
        }
        if segments.len() == 4 && segments[3] == "bindings" {
            match request.method() {
                Method::Get => {
                    return Ok((200, json_string(&state.handle.agent_bindings(&intent_id)?)?));
                }
                Method::Post => {
                    check_expected_revision(request, &state.handle.projection(&intent_id)?)?;
                    let value: Value = serde_json::from_str(body).map_err(json_error)?;
                    let scope_id = value["scope_id"]
                        .as_str()
                        .map(else_protocol::ids::ScopeId::from)
                        .or_else(|| {
                            state
                                .handle
                                .projection(&intent_id)
                                .ok()
                                .and_then(|projection| projection.root_scope_id)
                        })
                        .ok_or_else(|| DaemonError::InvalidRequest("missing scope_id".into()))?;
                    let binding = state.handle.bind_agent(
                        &intent_id,
                        else_protocol::ids::AgentId::from(value["agent_id"].as_str().ok_or_else(
                            || DaemonError::InvalidRequest("missing agent_id".into()),
                        )?),
                        value["name"].as_str().unwrap_or("agent"),
                        scope_id,
                        value["run_id"]
                            .as_str()
                            .map(else_protocol::ids::RunId::from),
                        serde_json::from_value(value["outbound"].clone()).map_err(json_error)?,
                        value["capability_ids"]
                            .as_array()
                            .map(|ids| {
                                ids.iter()
                                    .filter_map(Value::as_str)
                                    .map(else_protocol::ids::CapabilityId::from)
                                    .collect()
                            })
                            .unwrap_or_default(),
                        value["ttl_seconds"].as_i64().unwrap_or(3600),
                    )?;
                    return Ok((200, json_string(&binding)?));
                }
                _ => {}
            }
        }
        if segments.len() == 4 && segments[3] == "connectors" {
            match request.method() {
                Method::Get => {
                    let connectors = state
                        .handle
                        .projection(&intent_id)?
                        .connectors
                        .into_values()
                        .collect::<Vec<_>>();
                    return Ok((200, json_string(&connectors)?));
                }
                Method::Post => {
                    check_expected_revision(request, &state.handle.projection(&intent_id)?)?;
                    let value: Value = serde_json::from_str(body).map_err(json_error)?;
                    let name = value["name"].as_str().unwrap_or("connector");
                    let source = value["source"]
                        .as_str()
                        .ok_or_else(|| DaemonError::InvalidRequest("missing source".into()))?;
                    let adapter = value["adapter"]
                        .as_str()
                        .or_else(|| value["kind"].as_str())
                        .unwrap_or("calendar.ics");
                    let credential = value
                        .get("credential")
                        .filter(|credential| !credential.is_null())
                        .cloned()
                        .map(serde_json::from_value)
                        .transpose()
                        .map_err(json_error)?;
                    let sync_interval_seconds =
                        value["sync_interval_seconds"].as_u64().unwrap_or(300);
                    let connector = state.handle.register_connector_adapter_with_credential(
                        &intent_id,
                        name,
                        source,
                        adapter,
                        credential,
                        sync_interval_seconds,
                    )?;
                    return Ok((200, json_string(&connector)?));
                }
                _ => {}
            }
        }
        if segments.len() == 4 && segments[3] == "actions" {
            match request.method() {
                Method::Get => {
                    let actions = state.handle.connector_actions(&intent_id)?;
                    return Ok((200, json_string(&actions)?));
                }
                Method::Post => {
                    check_expected_revision(request, &state.handle.projection(&intent_id)?)?;
                    let value: Value = serde_json::from_str(body).map_err(json_error)?;
                    let connector_id = else_protocol::ids::ConnectorId::from(
                        value["connector_id"].as_str().ok_or_else(|| {
                            DaemonError::InvalidRequest("missing connector_id".into())
                        })?,
                    );
                    let capability = value["capability"]
                        .as_str()
                        .ok_or_else(|| DaemonError::InvalidRequest("missing capability".into()))?;
                    let target = value["target"]
                        .as_str()
                        .ok_or_else(|| DaemonError::InvalidRequest("missing target".into()))?;
                    let action = state.handle.propose_connector_action(
                        &intent_id,
                        &connector_id,
                        capability,
                        target,
                        value["input"].clone(),
                        value["idempotency_key"].as_str().map(str::to_string),
                    )?;
                    return Ok((200, json_string(&action)?));
                }
                _ => {}
            }
        }
        if segments.len() == 7
            && segments[3] == "connectors"
            && segments[5] == "actions"
            && segments[6] == "preview"
            && request.method() == &Method::Post
        {
            let value: Value = serde_json::from_str(body).map_err(json_error)?;
            let connector_id = else_protocol::ids::ConnectorId::from(segments[4]);
            let capability = value["capability"]
                .as_str()
                .ok_or_else(|| DaemonError::InvalidRequest("missing capability".into()))?;
            let target = value["target"]
                .as_str()
                .ok_or_else(|| DaemonError::InvalidRequest("missing target".into()))?;
            let preview = state.handle.preview_connector_action(
                &intent_id,
                &connector_id,
                capability,
                target,
                value["input"].clone(),
            )?;
            return Ok((200, json_string(&preview)?));
        }
        if segments.len() == 6
            && segments[3] == "actions"
            && matches!(segments[5], "execute" | "verify" | "compensate")
            && request.method() == &Method::Post
        {
            check_expected_revision(request, &state.handle.projection(&intent_id)?)?;
            let action_id = else_protocol::ids::ConnectorActionId::from(segments[4]);
            let action = match segments[5] {
                "execute" => state
                    .handle
                    .execute_connector_action(&intent_id, &action_id)?,
                "verify" => state
                    .handle
                    .verify_connector_action(&intent_id, &action_id)?,
                "compensate" => state
                    .handle
                    .compensate_connector_action(&intent_id, &action_id)?,
                _ => unreachable!("route guard"),
            };
            return Ok((200, json_string(&action)?));
        }
        if segments.len() == 4
            && segments[3] == "connector-policy"
            && request.method() == &Method::Post
        {
            check_expected_revision(request, &state.handle.projection(&intent_id)?)?;
            let policy: else_protocol::connector::ConnectorPolicy =
                serde_json::from_str(body).map_err(json_error)?;
            let spec = state
                .handle
                .configure_connector_policy(&intent_id, policy)?;
            return Ok((200, json_string(&spec)?));
        }
        if segments.len() == 4 && segments[3] == "signals" && request.method() == &Method::Get {
            let signals = state
                .handle
                .projection(&intent_id)?
                .world_signals
                .into_values()
                .collect::<Vec<_>>();
            return Ok((200, json_string(&signals)?));
        }
        if segments.len() == 4 && segments[3] == "world" && request.method() == &Method::Get {
            let include_resolved = query_value(request.url(), "include_resolved")
                .is_some_and(|value| matches!(value.as_str(), "1" | "true" | "yes"));
            let items = state.handle.world_items(&intent_id, include_resolved)?;
            return Ok((200, json_string(&items)?));
        }
        if segments.len() == 4 && segments[3] == "projects" && request.method() == &Method::Get {
            let include_resolved = query_value(request.url(), "include_resolved")
                .is_some_and(|value| matches!(value.as_str(), "1" | "true" | "yes"));
            let projects = state
                .handle
                .project_worlds(Some(&intent_id), include_resolved)?;
            return Ok((200, json_string(&projects)?));
        }
        if segments.len() == 5
            && segments[3] == "world"
            && segments[4] == "resolve"
            && request.method() == &Method::Post
        {
            check_expected_revision(request, &state.handle.projection(&intent_id)?)?;
            let value: Value = serde_json::from_str(body).map_err(json_error)?;
            let item_id = else_protocol::ids::WorldItemId::from(
                value["item_id"]
                    .as_str()
                    .ok_or_else(|| DaemonError::InvalidRequest("missing item_id".into()))?,
            );
            let kind: else_protocol::world::WorldItemResolutionKind =
                serde_json::from_value(value["kind"].clone()).map_err(json_error)?;
            let note = value["note"].as_str().map(str::to_string);
            let item = state
                .handle
                .resolve_world_item(&intent_id, &item_id, kind, note)?;
            return Ok((200, json_string(&item)?));
        }
        if segments.len() == 5
            && segments[3] == "world"
            && segments[4] == "corrections"
            && request.method() == &Method::Post
        {
            check_expected_revision(request, &state.handle.projection(&intent_id)?)?;
            let correction: else_protocol::world::WorldCorrelationCorrection =
                serde_json::from_str(body).map_err(json_error)?;
            let correction = state
                .handle
                .correct_world_correlation(&intent_id, correction)?;
            return Ok((200, json_string(&correction)?));
        }
        if segments.len() == 5
            && segments[3] == "signals"
            && segments[4] == "resolve"
            && request.method() == &Method::Post
        {
            check_expected_revision(request, &state.handle.projection(&intent_id)?)?;
            let candidates = state.handle.resolve_signals(&intent_id)?;
            return Ok((200, json_string(&candidates)?));
        }
        if segments.len() == 4 && segments[3] == "candidates" {
            match request.method() {
                Method::Get => {
                    let candidates = state
                        .handle
                        .projection(&intent_id)?
                        .intent_candidates
                        .into_values()
                        .collect::<Vec<_>>();
                    return Ok((200, json_string(&candidates)?));
                }
                Method::Post => {
                    check_expected_revision(request, &state.handle.projection(&intent_id)?)?;
                    let candidate: else_protocol::connector::IntentCandidate =
                        serde_json::from_str(body).map_err(json_error)?;
                    let candidate = state
                        .handle
                        .propose_intent_candidate(&intent_id, candidate)?;
                    return Ok((200, json_string(&candidate)?));
                }
                _ => {}
            }
        }
        if segments.len() == 6
            && segments[3] == "candidates"
            && matches!(segments[5], "accept" | "reject")
            && request.method() == &Method::Post
        {
            check_expected_revision(request, &state.handle.projection(&intent_id)?)?;
            let candidate_id = else_protocol::ids::IntentCandidateId::from(segments[4]);
            let intent = state.handle.resolve_intent_candidate(
                &intent_id,
                &candidate_id,
                segments[5] == "accept",
            )?;
            let candidate = state
                .handle
                .projection(&intent_id)?
                .intent_candidates
                .get(&candidate_id)
                .cloned()
                .ok_or_else(|| DaemonError::InvalidRequest("candidate disappeared".into()))?;
            return Ok((
                200,
                json_string(&json!({"candidate":candidate,"intent":intent}))?,
            ));
        }
        if segments.len() == 6
            && segments[3] == "connectors"
            && segments[5] == "health"
            && request.method() == &Method::Get
        {
            let connector_id = else_protocol::ids::ConnectorId::from(segments[4]);
            let health = state.handle.connector_health(&intent_id, &connector_id)?;
            return Ok((200, json_string(&health)?));
        }
        if segments.len() == 6 && segments[3] == "connectors" && segments[5] == "credential" {
            let connector_id = else_protocol::ids::ConnectorId::from(segments[4]);
            match request.method() {
                Method::Post => {
                    check_expected_revision(request, &state.handle.projection(&intent_id)?)?;
                    let credential: else_protocol::connector::ConnectorCredential =
                        serde_json::from_str(body).map_err(json_error)?;
                    let metadata = state.handle.attach_connector_credential(
                        &intent_id,
                        &connector_id,
                        credential,
                    )?;
                    return Ok((200, json_string(&metadata)?));
                }
                Method::Delete => {
                    check_expected_revision(request, &state.handle.projection(&intent_id)?)?;
                    let metadata = state
                        .handle
                        .revoke_connector_credential(&intent_id, &connector_id)?;
                    return Ok((200, json_string(&metadata)?));
                }
                _ => {}
            }
        }
        if segments.len() == 6
            && segments[3] == "connectors"
            && segments[5] == "cursor"
            && request.method() == &Method::Get
        {
            let connector_id = else_protocol::ids::ConnectorId::from(segments[4]);
            let cursors = state
                .handle
                .projection(&intent_id)?
                .connector_cursors
                .into_iter()
                .filter(|((id, _), _)| id == &connector_id)
                .map(|(_, cursor)| cursor)
                .collect::<Vec<_>>();
            return Ok((200, json_string(&cursors)?));
        }
        if segments.len() == 6
            && segments[3] == "connectors"
            && segments[5] == "sync"
            && request.method() == &Method::Post
        {
            check_expected_revision(request, &state.handle.projection(&intent_id)?)?;
            let result = state.handle.sync_connector(
                &intent_id,
                &else_protocol::ids::ConnectorId::from(segments[4]),
            )?;
            return Ok((200, json_string(&result)?));
        }
        if segments.len() == 6
            && segments[3] == "connectors"
            && segments[5] == "repair"
            && request.method() == &Method::Post
        {
            check_expected_revision(request, &state.handle.projection(&intent_id)?)?;
            let report = state.handle.repair_connector(
                &intent_id,
                &else_protocol::ids::ConnectorId::from(segments[4]),
            )?;
            return Ok((200, json_string(&report)?));
        }
        if segments.len() == 5
            && segments[3] == "mcp"
            && segments[4] == "discover"
            && request.method() == &Method::Post
        {
            check_expected_revision(request, &state.handle.projection(&intent_id)?)?;
            let value: Value = serde_json::from_str(body).map_err(json_error)?;
            let server_name = value["server_name"]
                .as_str()
                .ok_or_else(|| DaemonError::InvalidRequest("missing server_name".into()))?;
            let command = value["command"]
                .as_str()
                .ok_or_else(|| DaemonError::InvalidRequest("missing command".into()))?;
            return Ok((
                200,
                json_string(
                    &state
                        .handle
                        .discover_mcp_tools(&intent_id, server_name, command)?,
                )?,
            ));
        }
        if segments.len() == 6
            && segments[3] == "bindings"
            && segments[5] == "revoke"
            && request.method() == &Method::Post
        {
            check_expected_revision(request, &state.handle.projection(&intent_id)?)?;
            let value: Value = serde_json::from_str(body).unwrap_or_else(|_| json!({}));
            state.handle.revoke_binding(
                &intent_id,
                &else_protocol::ids::AgentBindingId::from(segments[4]),
                value["reason"].as_str().unwrap_or("surface revoke"),
            )?;
            return Ok((200, json!({"ok": true}).to_string()));
        }
        if segments.len() == 6
            && segments[3] == "bindings"
            && segments[5] == "dispatch"
            && request.method() == &Method::Post
        {
            check_expected_revision(request, &state.handle.projection(&intent_id)?)?;
            let projection = state.handle.dispatch_agent_binding(
                &intent_id,
                &else_protocol::ids::AgentBindingId::from(segments[4]),
            )?;
            return Ok((200, json_string(&snapshot(&projection))?));
        }
        if segments.len() == 6
            && segments[3] == "bindings"
            && segments[5] == "events"
            && request.method() == &Method::Get
        {
            let cursor = query_value(request.url(), "cursor")
                .and_then(|value| value.parse::<u64>().ok())
                .unwrap_or(0);
            let binding_id = else_protocol::ids::AgentBindingId::from(segments[4]);
            let events = state
                .handle
                .projection(&intent_id)?
                .agent_binding_events
                .get(&binding_id)
                .into_iter()
                .flatten()
                .filter(|(seq, _)| *seq > cursor)
                .map(|(seq, event)| json!({"seq":seq,"event":event}))
                .collect::<Vec<_>>();
            return Ok((200, json_string(&events)?));
        }
        if segments.len() == 6
            && segments[3] == "bindings"
            && segments[5] == "state"
            && request.method() == &Method::Post
        {
            check_expected_revision(request, &state.handle.projection(&intent_id)?)?;
            let value: Value = serde_json::from_str(body).map_err(json_error)?;
            let state_value: else_protocol::capability::AgentBindingState =
                serde_json::from_value(value["state"].clone()).map_err(json_error)?;
            state.handle.transition_agent_binding(
                &intent_id,
                &else_protocol::ids::AgentBindingId::from(segments[4]),
                state_value,
            )?;
            return Ok((200, json!({"ok":true}).to_string()));
        }
        if segments.len() == 6
            && segments[3] == "decisions"
            && segments[5] == "resolve"
            && request.method() == &Method::Post
        {
            check_expected_revision(request, &state.handle.projection(&intent_id)?)?;
            let value: Value = serde_json::from_str(body).map_err(json_error)?;
            let choice: DecisionChoice =
                serde_json::from_value(value["choice"].clone()).map_err(json_error)?;
            let decision_id = else_protocol::ids::AttentionId::from(segments[4]);
            let mut current = snapshot(&state.handle.projection(&intent_id)?);
            current.operation_id = Some(spawn_operation(state, "decision", &intent_id, {
                let handle = state.handle.clone();
                let intent_id = intent_id.clone();
                move || {
                    handle
                        .decide(&intent_id, &decision_id, choice)
                        .map(|_| json!({}))
                }
            }));
            return Ok((202, json_string(&current)?));
        }
        if segments.len() == 4 && segments[3] == "abort" && request.method() == &Method::Post {
            check_expected_revision(request, &state.handle.projection(&intent_id)?)?;
            let value: Value = serde_json::from_str(body).unwrap_or_else(|_| json!({}));
            let reason = value["reason"].as_str().unwrap_or("surface abort");
            let projection = state.handle.abort(&intent_id, reason)?;
            return Ok((200, json_string(&snapshot(&projection))?));
        }
        if segments.len() == 4 && segments[3] == "advance" && request.method() == &Method::Post {
            check_expected_revision(request, &state.handle.projection(&intent_id)?)?;
            let value: Value = serde_json::from_str(body).unwrap_or_else(|_| json!({}));
            let harness = value["harness"].as_str().map(str::to_string);
            let mut current = snapshot(&state.handle.projection(&intent_id)?);
            current.operation_id = Some(spawn_operation(state, "advance", &intent_id, {
                let handle = state.handle.clone();
                let intent_id = intent_id.clone();
                move || {
                    handle
                        .advance(&intent_id, harness.as_deref())
                        .map(|_| json!({}))
                }
            }));
            return Ok((202, json_string(&current)?));
        }
        if segments.len() == 4 && segments[3] == "drive" && request.method() == &Method::Post {
            check_expected_revision(request, &state.handle.projection(&intent_id)?)?;
            let value: Value = serde_json::from_str(body).unwrap_or_else(|_| json!({}));
            let harness = value["harness"].as_str().map(str::to_string);
            let max_steps = value["max_steps"].as_u64().unwrap_or(8);
            let mut current = snapshot(&state.handle.projection(&intent_id)?);
            current.operation_id = Some(spawn_operation(state, "drive", &intent_id, {
                let handle = state.handle.clone();
                let intent_id = intent_id.clone();
                move || {
                    handle
                        .drive(&intent_id, harness.as_deref(), max_steps)
                        .map(|driven| {
                            json!({
                                "stopped": format!("{:?}", driven.stopped),
                                "steps": driven.steps,
                            })
                        })
                }
            }));
            return Ok((202, json_string(&current)?));
        }
        if segments.len() == 4 && segments[3] == "scopes" {
            match request.method() {
                Method::Get => {
                    let projection = state.handle.projection(&intent_id)?;
                    let root = projection.spec.as_ref();
                    let views: Vec<_> = projection
                        .scopes
                        .iter()
                        .map(|(scope_id, scope)| scope.view(scope_id, root))
                        .collect();
                    return Ok((200, json_string(&views)?));
                }
                Method::Post => {
                    check_expected_revision(request, &state.handle.projection(&intent_id)?)?;
                    let value: Value = serde_json::from_str(body).map_err(json_error)?;
                    let parent_scope = value["parent_scope"]
                        .as_str()
                        .map(else_protocol::ids::ScopeId::from)
                        .or_else(|| {
                            state
                                .handle
                                .projection(&intent_id)
                                .ok()
                                .and_then(|projection| projection.root_scope_id)
                        })
                        .ok_or_else(|| {
                            DaemonError::InvalidRequest("missing parent_scope".into())
                        })?;
                    let task = value["task"]
                        .as_str()
                        .ok_or_else(|| DaemonError::InvalidRequest("missing task".into()))?;
                    let depends_on = value["depends_on"]
                        .as_array()
                        .map(|ids| {
                            ids.iter()
                                .filter_map(Value::as_str)
                                .map(else_protocol::ids::ScopeId::from)
                                .collect()
                        })
                        .unwrap_or_default();
                    let max_effect = parse_side_effect(value["max_effect"].as_str());
                    let scope_id = state.handle.spawn_subagent_with_dependencies(
                        &intent_id,
                        &parent_scope,
                        task,
                        max_effect,
                        depends_on,
                    )?;
                    return Ok((
                        200,
                        json_string(&json!({ "scope_id": scope_id.to_string() }))?,
                    ));
                }
                _ => {}
            }
        }
        if segments.len() == 6
            && segments[3] == "scopes"
            && segments[5] == "run"
            && request.method() == &Method::Post
        {
            check_expected_revision(request, &state.handle.projection(&intent_id)?)?;
            let scope_id = else_protocol::ids::ScopeId::from(segments[4]);
            let value: Value = serde_json::from_str(body).unwrap_or_else(|_| json!({}));
            let harness = value["harness"].as_str().map(str::to_string);
            let mut current = snapshot(&state.handle.projection(&intent_id)?);
            current.operation_id = Some(spawn_operation(state, "run_scope", &intent_id, {
                let handle = state.handle.clone();
                let intent_id = intent_id.clone();
                move || {
                    handle
                        .dispatch_scope(&intent_id, &scope_id, harness.as_deref())
                        .map(|_| json!({}))
                }
            }));
            return Ok((202, json_string(&current)?));
        }
        if segments.len() == 4
            && segments[3] == "wait-external"
            && request.method() == &Method::Post
        {
            check_expected_revision(request, &state.handle.projection(&intent_id)?)?;
            let value: Value = serde_json::from_str(body).unwrap_or_else(|_| json!({}));
            let reason = value["reason"]
                .as_str()
                .unwrap_or("waiting for external event");
            let projection = state.handle.wait_for_external(&intent_id, reason)?;
            return Ok((200, json_string(&snapshot(&projection))?));
        }
        if segments.len() == 4 && segments[3] == "events" && request.method() == &Method::Post {
            check_expected_revision(request, &state.handle.projection(&intent_id)?)?;
            let value: Value = serde_json::from_str(body).map_err(json_error)?;
            let source = value["source"].as_str().unwrap_or("external");
            let summary = value["summary"].as_str().unwrap_or("");
            let projection = state
                .handle
                .record_external_event(&intent_id, source, summary)?;
            return Ok((200, json_string(&snapshot(&projection))?));
        }
        if segments.len() == 4 && segments[3] == "timers" && request.method() == &Method::Post {
            check_expected_revision(request, &state.handle.projection(&intent_id)?)?;
            let value: Value = serde_json::from_str(body).map_err(json_error)?;
            let seconds = value["seconds"].as_i64().unwrap_or(3600);
            let reason = value["reason"].as_str().unwrap_or("follow up");
            let fire_at = Utc::now() + chrono::Duration::seconds(seconds);
            let timer_id = state.handle.schedule_timer(&intent_id, fire_at, reason)?;
            return Ok((
                200,
                json_string(&json!({ "timer_id": timer_id.to_string() }))?,
            ));
        }
        if segments.len() == 4 && segments[3] == "plan" && request.method() == &Method::Get {
            let projection = state.handle.projection(&intent_id)?;
            let current = projection
                .current_plan_id
                .as_ref()
                .and_then(|plan_id| projection.plans.get(plan_id))
                .cloned();
            let view = else_protocol::api::PlanView {
                current,
                history: state.handle.plan_history(&intent_id)?,
                impacts: projection.plan_impacts.clone(),
            };
            return Ok((200, json_string(&view)?));
        }
        if segments.len() == 5
            && segments[3] == "plan"
            && segments[4] == "impact"
            && request.method() == &Method::Post
        {
            check_expected_revision(request, &state.handle.projection(&intent_id)?)?;
            let value: Value = serde_json::from_str(body).map_err(json_error)?;
            let source_event_id = value["source_event_id"].as_str().unwrap_or("");
            let source = value["source"].as_str().unwrap_or("");
            let summary = value["summary"].as_str().unwrap_or("");
            let impact =
                state
                    .handle
                    .analyze_plan_impact(&intent_id, source_event_id, source, summary)?;
            return Ok((200, json_string(&impact)?));
        }
        if segments.len() == 4 && segments[3] == "attention" && request.method() == &Method::Get {
            let projection = state.handle.projection(&intent_id)?;
            let now = Utc::now();
            let view = else_protocol::api::AttentionView {
                metrics: projection.attention_metrics(now),
                windows: projection.attention_windows.values().cloned().collect(),
                requests: projection
                    .actionable_requests(now)
                    .into_iter()
                    .cloned()
                    .collect(),
            };
            return Ok((200, json_string(&view)?));
        }
        if segments.len() == 4 && segments[3] == "memory" && request.method() == &Method::Get {
            let view = state.handle.memory_projection()?.view();
            return Ok((200, json_string(&view)?));
        }
        if segments.len() == 5 && segments[3] == "memory" && request.method() == &Method::Post {
            let value: Value = serde_json::from_str(body).map_err(json_error)?;
            match segments[4] {
                "promote" => {
                    let candidate_id = else_protocol::ids::MemoryId::from(
                        value["candidate_id"].as_str().ok_or_else(|| {
                            DaemonError::InvalidRequest("missing candidate_id".into())
                        })?,
                    );
                    let scope = match value["scope"].as_str() {
                        Some("global") => Some(else_protocol::memory::MemoryScope::Global),
                        Some(raw) if raw.starts_with("project:") => {
                            Some(else_protocol::memory::MemoryScope::Project(
                                raw.trim_start_matches("project:").to_string(),
                            ))
                        }
                        Some(raw) if raw.starts_with("intent:") => {
                            Some(else_protocol::memory::MemoryScope::Intent(
                                else_protocol::ids::IntentId::from(
                                    raw.trim_start_matches("intent:"),
                                ),
                            ))
                        }
                        _ => None,
                    };
                    let record = state.handle.promote_memory(&candidate_id, scope)?;
                    return Ok((200, json_string(&record)?));
                }
                "revoke" => {
                    let memory_id = else_protocol::ids::MemoryId::from(
                        value["memory_id"].as_str().ok_or_else(|| {
                            DaemonError::InvalidRequest("missing memory_id".into())
                        })?,
                    );
                    let reason = value["reason"].as_str().unwrap_or("用户撤销");
                    state.handle.revoke_memory(&memory_id, reason)?;
                    return Ok((
                        200,
                        json_string(&json!({ "revoked": memory_id.to_string() }))?,
                    ));
                }
                "correct" => {
                    let project = value["project"]
                        .as_str()
                        .ok_or_else(|| DaemonError::InvalidRequest("missing project".into()))?;
                    let subject = value["subject"]
                        .as_str()
                        .ok_or_else(|| DaemonError::InvalidRequest("missing subject".into()))?;
                    let predicate = value["predicate"].as_str().unwrap_or("user_correction");
                    let record = value["value"].as_str().unwrap_or("");
                    let memory = state.handle.correct_memory(
                        else_protocol::memory::MemoryScope::Project(project.to_string()),
                        subject,
                        predicate,
                        record,
                    )?;
                    return Ok((200, json_string(&memory)?));
                }
                _ => {}
            }
        }
        if segments.len() == 4 && segments[3] == "checkpoints" && request.method() == &Method::Post
        {
            check_expected_revision(request, &state.handle.projection(&intent_id)?)?;
            let checkpoint = state.handle.checkpoint(&intent_id)?;
            return Ok((200, json_string(&checkpoint)?));
        }
        if segments.len() == 4 && segments[3] == "rerun" && request.method() == &Method::Post {
            check_expected_revision(request, &state.handle.projection(&intent_id)?)?;
            let value: Value = serde_json::from_str(body).map_err(json_error)?;
            let checkpoint_id = else_protocol::ids::CheckpointId::from(
                value["checkpoint_id"]
                    .as_str()
                    .ok_or_else(|| DaemonError::InvalidRequest("missing checkpoint_id".into()))?,
            );
            let alternative = value["alternative"].as_bool().unwrap_or(false);
            let (scope_id, generation) =
                state
                    .handle
                    .rerun(&intent_id, &checkpoint_id, alternative)?;
            return Ok((
                200,
                json_string(&json!({
                    "scope_id": scope_id.to_string(),
                    "generation": generation,
                }))?,
            ));
        }
        if segments.len() == 4 && segments[3] == "compare" && request.method() == &Method::Get {
            let comparison = state.handle.compare(&intent_id)?;
            return Ok((200, json_string(&json!({ "comparison": comparison }))?));
        }
        if segments.len() == 4 && segments[3] == "opaque" && request.method() == &Method::Post {
            check_expected_revision(request, &state.handle.projection(&intent_id)?)?;
            let value: Value = serde_json::from_str(body).map_err(json_error)?;
            let policy = else_protocol::intent::OpaqueHarnessPolicy {
                enabled: value["enabled"].as_bool().unwrap_or(true),
                allow_native_provider_network: value["allow_native_provider_network"]
                    .as_bool()
                    .unwrap_or(false),
                use_tool_gateway: value["use_tool_gateway"].as_bool().unwrap_or(false),
                max_observed_effect: parse_side_effect(value["max_observed_effect"].as_str()),
            };
            let spec = state.handle.configure_opaque_harness(&intent_id, policy)?;
            return Ok((200, json_string(&spec)?));
        }
        if segments.len() == 4 && segments[3] == "resources" && request.method() == &Method::Get {
            let report = state.handle.resources(&intent_id)?;
            return Ok((200, json_string(&report.view())?));
        }
        if segments.len() == 4 && segments[3] == "revise" && request.method() == &Method::Post {
            check_expected_revision(request, &state.handle.projection(&intent_id)?)?;
            let value: Value = serde_json::from_str(body).map_err(json_error)?;
            let update = else_api::IntentRevision {
                outcome: value["outcome"].as_str().map(str::to_string),
                add_constraints: value["add_constraints"]
                    .as_array()
                    .map(|items| {
                        items
                            .iter()
                            .filter_map(Value::as_str)
                            .map(str::to_string)
                            .collect()
                    })
                    .unwrap_or_default(),
                reason: value["reason"].as_str().unwrap_or("用户修订").to_string(),
            };
            let spec = state.handle.revise_intent(&intent_id, update, &[])?;
            return Ok((200, json_string(&spec)?));
        }
        if segments.len() == 3 && segments[2].starts_with("operations") {
            // handled below
        }
    }
    if segments.len() == 3
        && segments[0] == "v1"
        && segments[1] == "operations"
        && request.method() == &Method::Get
    {
        if let Some(operation) = state
            .operations
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .get(segments[2])
            .cloned()
        {
            return Ok((200, json_string(&operation)?));
        }
        return Err(DaemonError::InvalidRequest("operation not found".into()));
    }
    if segments == ["v1", "events"] && request.method() == &Method::Get {
        let cursor = query_value(request.url(), "cursor")
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(0);
        let mut events = Vec::new();
        for spec in state.handle.list_intents()? {
            events.extend(state.handle.events(&spec.id)?);
        }
        events.sort_by_key(|event| event.seq);
        let events: Vec<_> = events
            .into_iter()
            .filter(|event| event.seq > cursor)
            .map(event_response)
            .collect();
        return Ok((200, json_string(&events)?));
    }
    Ok((404, json!({ "error": "not found" }).to_string()))
}

fn spawn_operation(
    state: &AppState,
    kind: &str,
    intent_id: &else_protocol::ids::IntentId,
    operation: impl FnOnce() -> else_api::Result<Value> + Send + 'static,
) -> String {
    let id = uuid::Uuid::new_v4().to_string();
    let record = Operation {
        id: id.clone(),
        intent_id: intent_id.to_string(),
        kind: kind.into(),
        status: "running".into(),
        error: None,
        result: None,
    };
    state
        .operations
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
        .insert(id.clone(), record);
    let state_operations = state.operations.clone();
    let thread_id = id.clone();
    thread::spawn(move || {
        let result = operation();
        if let Some(record) = state_operations
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .get_mut(&thread_id)
        {
            match result {
                Ok(value) => {
                    record.status = "completed".into();
                    record.result = Some(value);
                }
                Err(error) => {
                    record.status = "failed".into();
                    record.error = Some(error.to_string());
                }
            }
        }
    });
    id
}

fn check_expected_revision(request: &Request, projection: &IntentProjection) -> Result<()> {
    let Some(expected) = header(request, "If-Match") else {
        return Err(DaemonError::InvalidRequest(
            "If-Match revision is required".into(),
        ));
    };
    let expected = expected.trim_matches('"');
    let actual = projection
        .spec
        .as_ref()
        .map(|spec| spec.revision)
        .unwrap_or(0);
    if expected != actual.to_string() {
        return Err(DaemonError::Api(ApiError::Conflict(format!(
            "expected revision {expected}, actual {actual}"
        ))));
    }
    Ok(())
}

fn snapshot(projection: &IntentProjection) -> IntentSnapshot {
    IntentSnapshot {
        spec: projection.spec.clone().unwrap_or_else(|| {
            let id = else_protocol::ids::IntentId::from("unknown");
            IntentSpec {
                id,
                revision: 0,
                outcome: String::new(),
                done_when: vec![],
                constraints: vec![],
                authority: else_protocol::intent::Authority {
                    max_autonomous_effect: else_protocol::effect::SideEffectKind::ReadOnly,
                    forbidden: vec![],
                },
                attention_policy: else_protocol::intent::AttentionPolicy {
                    allow_interrupt: true,
                    batch_interval_minutes: 30,
                },
                opaque_harness_policy: Default::default(),
                deadline: None,
                assumptions: vec![],
                project: None,
                origin: None,
                connector_policy: Default::default(),
            }
        }),
        state: projection.state.unwrap_or(IntentState::Active),
        decisions: projection
            .actionable_requests(Utc::now())
            .into_iter()
            .cloned()
            .collect(),
        artifacts: projection.artifacts.clone(),
        resource_budget: projection.resource_budget,
        resource_usage: projection.resource_usage,
        cursor: projection.cursor,
        operation_id: None,
    }
}

fn event_response(event: StoredEvent) -> EventResponse {
    EventResponse {
        seq: event.seq,
        event_id: event.envelope.event_id,
        intent_id: event.envelope.intent_id.to_string(),
        scope_id: event.envelope.scope_id.to_string(),
        run_id: event.envelope.run_id.map(|id| id.to_string()),
        generation: event.envelope.generation,
        intent_revision: event.envelope.intent_revision,
        occurred_at: event.envelope.occurred_at.to_rfc3339(),
        payload: serde_json::to_value(event.envelope.payload).unwrap_or(Value::Null),
    }
}

fn authorized(request: &Request, token: &str) -> bool {
    header(request, "Authorization").is_some_and(|value| value == format!("Bearer {token}"))
}

fn header(request: &Request, name: &str) -> Option<String> {
    request
        .headers()
        .iter()
        .find(|header| header.field.to_string().eq_ignore_ascii_case(name))
        .map(|header| header.value.as_str().to_string())
}

fn query_value(url: &str, key: &str) -> Option<String> {
    url.split_once('?')?.1.split('&').find_map(|pair| {
        let (name, value) = pair.split_once('=')?;
        (name == key).then(|| percent_decode(value))
    })
}

fn percent_decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'+' => {
                decoded.push(b' ');
                index += 1;
            }
            b'%' if index + 2 < bytes.len() => {
                let high = (bytes[index + 1] as char).to_digit(16);
                let low = (bytes[index + 2] as char).to_digit(16);
                if let (Some(high), Some(low)) = (high, low) {
                    decoded.push((high * 16 + low) as u8);
                    index += 3;
                } else {
                    decoded.push(bytes[index]);
                    index += 1;
                }
            }
            byte => {
                decoded.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8_lossy(&decoded).into_owned()
}

fn respond_json(request: Request, status: u16, value: &Value) {
    respond(request, status, &value.to_string());
}

fn respond(request: Request, status: u16, body: &str) {
    let mut response = Response::from_string(body.to_string()).with_status_code(StatusCode(status));
    if let Ok(header) = Header::from_bytes("Content-Type", "application/json") {
        response.add_header(header);
    }
    let _ = request.respond(response);
}

fn json_string(value: &impl Serialize) -> Result<String> {
    serde_json::to_string(value).map_err(json_error)
}

fn json_error(error: serde_json::Error) -> DaemonError {
    DaemonError::InvalidRequest(error.to_string())
}

/// 解析 surface 传来的副作用等级。
///
/// 缺省是 `ReadOnly`：未知或缺失的等级一律按最保守的解释处理，
/// 不因为解析失败而放大权限。
fn parse_side_effect(raw: Option<&str>) -> else_protocol::effect::SideEffectKind {
    match raw {
        Some("read_only") => else_protocol::effect::SideEffectKind::ReadOnly,
        Some("reversible") => else_protocol::effect::SideEffectKind::Reversible,
        Some("compensatable") => else_protocol::effect::SideEffectKind::Compensatable,
        Some("irreversible") => else_protocol::effect::SideEffectKind::Irreversible,
        Some("unknown") => else_protocol::effect::SideEffectKind::Unknown,
        _ => else_protocol::effect::SideEffectKind::ReadOnly,
    }
}

struct BindingEventStream {
    handle: RuntimeHandle,
    intent_id: else_protocol::ids::IntentId,
    binding_id: else_protocol::ids::AgentBindingId,
    cursor: u64,
    buffer: Vec<u8>,
    position: usize,
    deadline: Instant,
}

impl BindingEventStream {
    fn new(
        handle: RuntimeHandle,
        intent_id: else_protocol::ids::IntentId,
        binding_id: else_protocol::ids::AgentBindingId,
        cursor: u64,
    ) -> Self {
        Self {
            handle,
            intent_id,
            binding_id,
            cursor,
            buffer: Vec::new(),
            position: 0,
            deadline: Instant::now() + Duration::from_secs(60),
        }
    }
}

impl Read for BindingEventStream {
    fn read(&mut self, output: &mut [u8]) -> std::io::Result<usize> {
        loop {
            if self.position < self.buffer.len() {
                let available = &self.buffer[self.position..];
                let count = available.len().min(output.len());
                output[..count].copy_from_slice(&available[..count]);
                self.position += count;
                return Ok(count);
            }
            if Instant::now() >= self.deadline {
                return Ok(0);
            }
            self.buffer.clear();
            self.position = 0;
            if let Ok(projection) = self.handle.projection(&self.intent_id) {
                if let Some(events) = projection.agent_binding_events.get(&self.binding_id) {
                    let cursor = self.cursor;
                    for (seq, event) in events.iter().filter(|(seq, _)| *seq > cursor) {
                        self.cursor = *seq;
                        let payload = serde_json::to_string(&json!({
                            "seq":seq,
                            "event":event
                        }))
                        .unwrap_or_else(|_| "{}".into());
                        self.buffer.extend_from_slice(b"data: ");
                        self.buffer.extend_from_slice(payload.as_bytes());
                        self.buffer.extend_from_slice(b"\n\n");
                    }
                    if !self.buffer.is_empty() {
                        continue;
                    }
                }
            }
            std::thread::sleep(Duration::from_millis(250));
        }
    }
}

/// 全局事件流：把所有 Intent 的新事件按 `seq` 推给客户端。
///
/// surface（TUI、未来的 Web）用它做增量刷新：只重取变化的部分，
/// 不必轮询整份快照。客户端用 `cursor` 续读；流在 60 秒后关闭，
/// 客户端重连时带上新的 `cursor` 即可。
struct EventStream {
    handle: RuntimeHandle,
    cursor: u64,
    buffer: Vec<u8>,
    position: usize,
    deadline: Instant,
}

impl EventStream {
    fn new(handle: RuntimeHandle, cursor: u64) -> Self {
        Self {
            handle,
            cursor,
            buffer: Vec::new(),
            position: 0,
            deadline: Instant::now() + Duration::from_secs(60),
        }
    }
}

impl Read for EventStream {
    fn read(&mut self, output: &mut [u8]) -> std::io::Result<usize> {
        loop {
            if self.position < self.buffer.len() {
                let available = &self.buffer[self.position..];
                let count = available.len().min(output.len());
                output[..count].copy_from_slice(&available[..count]);
                self.position += count;
                return Ok(count);
            }
            if Instant::now() >= self.deadline {
                return Ok(0);
            }
            self.buffer.clear();
            self.position = 0;
            let mut events = Vec::new();
            if let Ok(specs) = self.handle.list_intents() {
                for spec in specs {
                    if let Ok(mut intent_events) = self.handle.events(&spec.id) {
                        events.append(&mut intent_events);
                    }
                }
            }
            events.sort_by_key(|event| event.seq);
            let cursor = self.cursor;
            for event in events.into_iter().filter(|event| event.seq > cursor) {
                self.cursor = event.seq;
                let payload =
                    serde_json::to_string(&event_response(event)).unwrap_or_else(|_| "{}".into());
                self.buffer.extend_from_slice(b"data: ");
                self.buffer.extend_from_slice(payload.as_bytes());
                self.buffer.extend_from_slice(b"\n\n");
            }
            if !self.buffer.is_empty() {
                continue;
            }
            std::thread::sleep(Duration::from_millis(250));
        }
    }
}

fn write_token(dir: &Path, token: &str) -> Result<()> {
    let path = dir.join("daemon-token");
    let temporary = dir.join(format!(".daemon-token-{}", uuid::Uuid::new_v4()));
    let mut options = OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let result = (|| {
        use std::io::Write;
        let mut file = options.open(&temporary)?;
        file.write_all(token.as_bytes())?;
        file.sync_all()?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&temporary, std::fs::Permissions::from_mode(0o600))?;
        }
        std::fs::rename(&temporary, &path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}

pub fn run_from_args() -> Result<()> {
    let mut dir = None;
    let mut bind = "127.0.0.1:4180".to_string();
    let mut token = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--dir" => dir = args.next().map(PathBuf::from),
            "--bind" => bind = args.next().unwrap_or(bind),
            "--token" => token = args.next(),
            "--help" | "-h" => {
                println!("else-daemon [--dir PATH] [--bind 127.0.0.1:4180] [--token TOKEN]");
                return Ok(());
            }
            _ => return Err(DaemonError::InvalidRequest(format!("unknown arg {arg}"))),
        }
    }
    let dir = dir
        .or_else(|| std::env::var_os("ELSE_RUNTIME_DIR").map(PathBuf::from))
        .unwrap_or_else(default_runtime_dir);
    let token_path = dir.join("daemon-token");
    let daemon = Daemon::start(DaemonConfig { dir, bind, token })?;
    println!("runtime: {}", daemon.url);
    println!("token-file: {}", token_path.display());
    loop {
        thread::park_timeout(Duration::from_secs(60));
    }
}

fn default_runtime_dir() -> PathBuf {
    #[cfg(target_os = "macos")]
    {
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home)
                .join("Library")
                .join("Application Support")
                .join("else")
                .join("runtime");
        }
    }
    PathBuf::from(".else")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpStream;

    fn request(
        daemon: &Daemon,
        method: &str,
        path: &str,
        body: &str,
        idempotency: Option<&str>,
        revision: Option<u64>,
    ) -> (u16, String) {
        let authority = daemon.url.trim_start_matches("http://");
        let mut stream = TcpStream::connect(authority).unwrap();
        let mut headers = format!(
            "{method} {path} HTTP/1.1\r\nHost: {authority}\r\nAuthorization: Bearer {}\r\nAccept: application/json\r\nContent-Length: {}\r\nConnection: close\r\n",
            daemon.token,
            body.len()
        );
        if let Some(key) = idempotency {
            headers.push_str(&format!("Idempotency-Key: {key}\r\n"));
        }
        if let Some(revision) = revision {
            headers.push_str(&format!("If-Match: {revision}\r\n"));
        }
        headers.push_str("\r\n");
        stream.write_all(headers.as_bytes()).unwrap();
        stream.write_all(body.as_bytes()).unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        let status = response
            .split_whitespace()
            .nth(1)
            .and_then(|value| value.parse().ok())
            .unwrap();
        let body = response
            .split_once("\r\n\r\n")
            .map(|(_, body)| body)
            .unwrap_or_default()
            .to_string();
        (status, body)
    }

    fn request_with_token(daemon: &Daemon, token: &str, path: &str, body: &str) -> (u16, String) {
        let authority = daemon.url.trim_start_matches("http://");
        let mut stream = TcpStream::connect(authority).unwrap();
        write!(
            stream,
            "POST {path} HTTP/1.1\r\nHost: {authority}\r\nAuthorization: Bearer {token}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
        .unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        let status = response
            .split_whitespace()
            .nth(1)
            .and_then(|value| value.parse().ok())
            .unwrap();
        let body = response
            .split_once("\r\n\r\n")
            .map(|(_, body)| body)
            .unwrap_or_default()
            .to_string();
        (status, body)
    }

    #[test]
    fn daemon_exposes_authenticated_intent_api_and_idempotency() {
        let dir = tempfile::tempdir().unwrap();
        let daemon = Daemon::start(DaemonConfig {
            dir: dir.path().to_path_buf(),
            bind: "127.0.0.1:0".into(),
            token: Some("test-token".into()),
        })
        .unwrap();

        let (status, body) = request(
            &daemon,
            "POST",
            "/v1/intents",
            r#"{"outcome":"daemon 测试","project":"test"}"#,
            Some("create-1"),
            None,
        );
        assert_eq!(status, 200, "{body}");
        let snapshot: Value = serde_json::from_str(&body).unwrap();
        let intent_id = snapshot["spec"]["id"].as_str().unwrap().to_string();
        let revision = snapshot["spec"]["revision"].as_u64().unwrap();
        let (status, duplicate) = request(
            &daemon,
            "POST",
            "/v1/intents",
            r#"{"outcome":"daemon 测试","project":"test"}"#,
            Some("create-1"),
            None,
        );
        assert_eq!(status, 200);
        assert_eq!(duplicate, body);

        let (status, list) = request(&daemon, "GET", "/v1/intents", "", None, None);
        assert_eq!(status, 200);
        assert_eq!(serde_json::from_str::<Vec<Value>>(&list).unwrap().len(), 1);

        let capabilities_path = format!("/v1/intents/{intent_id}/capabilities");
        let (status, capabilities) = request(&daemon, "GET", &capabilities_path, "", None, None);
        assert_eq!(status, 200);
        assert!(capabilities.contains("fs.write"));

        let bindings_path = format!("/v1/intents/{intent_id}/bindings");
        let binding_body = serde_json::json!({
            "agent_id": "agent-1",
            "name": "daemon-agent",
            "outbound": {
                "kind": "local_process",
                "value": {"command": "agent"}
            },
            "capability_ids": ["fs.write"],
            "ttl_seconds": 60
        })
        .to_string();
        let (status, binding) = request(
            &daemon,
            "POST",
            &bindings_path,
            &binding_body,
            Some("bind-1"),
            Some(revision),
        );
        assert_eq!(status, 200, "{binding}");
        assert!(binding.contains("inbound_session"));
        let binding_value: Value = serde_json::from_str(&binding).unwrap();
        let binding_id = binding_value["id"].as_str().unwrap();
        let inbound_session = binding_value["inbound_session"].as_str().unwrap();
        let event_body = serde_json::json!({
            "intent_id": intent_id,
            "binding_id": binding_id,
            "event": {"type": "output", "data": {"text": "remote callback"}}
        })
        .to_string();
        let (status, callback) =
            request_with_token(&daemon, inbound_session, "/v1/binding-events", &event_body);
        assert_eq!(status, 200, "{callback}");
        let binding_events_path = format!("/v1/intents/{intent_id}/bindings/{binding_id}/events");
        let (status, binding_events) =
            request(&daemon, "GET", &binding_events_path, "", None, None);
        assert_eq!(status, 200);
        assert!(binding_events.contains("remote callback"));
        let (status, bindings) = request(&daemon, "GET", &bindings_path, "", None, None);
        assert_eq!(status, 200);
        assert_eq!(
            serde_json::from_str::<Vec<Value>>(&bindings).unwrap().len(),
            1
        );

        let run_path = format!("/v1/intents/{intent_id}/run");
        let (status, run) = request(
            &daemon,
            "POST",
            &run_path,
            "",
            Some("run-1"),
            Some(revision),
        );
        assert_eq!(status, 202, "{run}");
        let operation_id = serde_json::from_str::<Value>(&run).unwrap()["operation_id"]
            .as_str()
            .unwrap()
            .to_string();
        let operation_path = format!("/v1/operations/{operation_id}");
        let mut operation = Value::Null;
        for _ in 0..50 {
            let (status, body) = request(&daemon, "GET", &operation_path, "", None, None);
            assert_eq!(status, 200);
            operation = serde_json::from_str(&body).unwrap();
            if operation["status"] != "running" {
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(operation["status"], "completed");

        let (status, events) = request(&daemon, "GET", "/v1/events?cursor=0", "", None, None);
        assert_eq!(status, 200);
        assert!(!serde_json::from_str::<Vec<Value>>(&events)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn daemon_rejects_missing_token_and_second_instance() {
        let dir = tempfile::tempdir().unwrap();
        let daemon = Daemon::start(DaemonConfig {
            dir: dir.path().to_path_buf(),
            bind: "127.0.0.1:0".into(),
            token: Some("secret".into()),
        })
        .unwrap();
        let authority = daemon.url.trim_start_matches("http://");
        let mut stream = TcpStream::connect(authority).unwrap();
        write!(
            stream,
            "GET /v1/intents HTTP/1.1\r\nHost: {authority}\r\nConnection: close\r\n\r\n"
        )
        .unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        assert!(response.starts_with("HTTP/1.1 401"));

        let second = Daemon::start(DaemonConfig {
            dir: dir.path().to_path_buf(),
            bind: "127.0.0.1:0".into(),
            token: Some("secret".into()),
        });
        assert!(matches!(second, Err(DaemonError::AlreadyRunning(_))));
    }

    #[test]
    fn daemon_exposes_signal_and_candidate_lineage_endpoints() {
        let dir = tempfile::tempdir().unwrap();
        let daemon = Daemon::start(DaemonConfig {
            dir: dir.path().to_path_buf(),
            bind: "127.0.0.1:0".into(),
            token: Some("signal-token".into()),
        })
        .unwrap();
        let (status, body) = request(
            &daemon,
            "POST",
            "/v1/intents",
            r#"{"outcome":"候选测试"}"#,
            Some("signal-create"),
            None,
        );
        assert_eq!(status, 200, "{body}");
        let snapshot: Value = serde_json::from_str(&body).unwrap();
        let intent_id = snapshot["spec"]["id"].as_str().unwrap().to_string();
        let revision = snapshot["spec"]["revision"].as_u64().unwrap();

        let signals_path = format!("/v1/intents/{intent_id}/signals");
        let (status, signals) = request(&daemon, "GET", &signals_path, "", None, None);
        assert_eq!(status, 200, "{signals}");
        assert_eq!(
            serde_json::from_str::<Vec<Value>>(&signals).unwrap().len(),
            0
        );

        let candidates_path = format!("/v1/intents/{intent_id}/candidates");
        let candidate_body = json!({
            "id": "candidate-api-1",
            "origin": "external_request",
            "proposed_outcome": "整理外部请求",
            "done_when": [{
                "description": "确认完成",
                "verification": {"kind": "manual"}
            }],
            "constraints": [],
            "authority": {
                "max_autonomous_effect": "read_only",
                "forbidden": []
            },
            "attention_policy": {
                "allow_interrupt": false,
                "batch_interval_minutes": 60
            },
            "deadline": null,
            "related_signal_ids": [],
            "related_intent_id": intent_id,
            "confidence": 60,
            "dedupe_key": "daemon-candidate-1",
            "state": "proposed",
            "accepted_intent_id": null
        })
        .to_string();
        let (status, candidate) = request(
            &daemon,
            "POST",
            &candidates_path,
            &candidate_body,
            Some("signal-propose"),
            Some(revision),
        );
        assert_eq!(status, 200, "{candidate}");
        assert!(candidate.contains("candidate-api-1"));

        let accept_path = format!("/v1/intents/{intent_id}/candidates/candidate-api-1/accept");
        let (status, accepted) = request(
            &daemon,
            "POST",
            &accept_path,
            "",
            Some("signal-accept"),
            Some(revision),
        );
        assert_eq!(status, 200, "{accepted}");
        let accepted: Value = serde_json::from_str(&accepted).unwrap();
        assert_eq!(accepted["candidate"]["state"].as_str(), Some("accepted"));
        assert_eq!(
            accepted["intent"]["origin"]["candidate_id"].as_str(),
            Some("candidate-api-1")
        );
        assert_eq!(
            accepted["intent"]["origin"]["kind"].as_str(),
            Some("external_request")
        );
        daemon.shutdown();
    }

    #[test]
    fn daemon_registers_linear_and_accepts_session_scoped_webhooks() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("linear.json");
        std::fs::write(&source, r#"{"issues":[]}"#).unwrap();
        let daemon = Daemon::start(DaemonConfig {
            dir: dir.path().to_path_buf(),
            bind: "127.0.0.1:0".into(),
            token: Some("linear-token".into()),
        })
        .unwrap();
        let (status, body) = request(
            &daemon,
            "POST",
            "/v1/intents",
            r#"{"outcome":"Linear webhook 测试"}"#,
            Some("linear-create"),
            None,
        );
        assert_eq!(status, 200, "{body}");
        let snapshot: Value = serde_json::from_str(&body).unwrap();
        let intent_id = snapshot["spec"]["id"].as_str().unwrap().to_string();
        let revision = snapshot["spec"]["revision"].as_u64().unwrap();
        let connectors_path = format!("/v1/intents/{intent_id}/connectors");
        let connector_body = json!({
            "kind": "linear",
            "name": "engineering",
            "source": source
        })
        .to_string();
        let (status, connector) = request(
            &daemon,
            "POST",
            &connectors_path,
            &connector_body,
            Some("linear-register"),
            Some(revision),
        );
        assert_eq!(status, 200, "{connector}");
        let connector: Value = serde_json::from_str(&connector).unwrap();
        let connector_id = connector["id"].as_str().unwrap();
        let session = connector["webhook_session"].as_str().unwrap();
        assert_eq!(connector["adapter"].as_str(), Some("linear.graphql"));
        assert_eq!(connector["manifest"]["provider"].as_str(), Some("linear"));
        let (status, adapters) = request(&daemon, "GET", "/v1/connector-adapters", "", None, None);
        assert_eq!(status, 200, "{adapters}");
        assert!(adapters.contains("calendar.ics"));
        assert!(adapters.contains("linear.graphql"));
        assert!(adapters.contains("notion.api"));

        let payload = json!({
            "id": "issue-webhook",
            "identifier": "ENG-200",
            "title": "等待合作方确认",
            "state": {"name": "Blocked", "type": "started"},
            "updatedAt": Utc::now().to_rfc3339(),
            "labels": {"nodes": [{"name": "waiting_external"}]},
            "blockedBy": {"nodes": [{"id": "external"}]}
        });
        let event_body = json!({
            "intent_id": intent_id,
            "connector_id": connector_id,
            "payload": {
                "action": "update",
                "type": "Issue",
                "data": payload
            }
        })
        .to_string();
        let (status, denied) = request_with_token(
            &daemon,
            "wrong-session",
            "/v1/connector-events",
            &event_body,
        );
        assert_eq!(status, 403, "{denied}");
        let (status, accepted) =
            request_with_token(&daemon, session, "/v1/connector-events", &event_body);
        assert_eq!(status, 200, "{accepted}");
        let signals_path = format!("/v1/intents/{intent_id}/signals");
        let (status, signals) = request(&daemon, "GET", &signals_path, "", None, None);
        assert_eq!(status, 200, "{signals}");
        assert_eq!(
            serde_json::from_str::<Vec<Value>>(&signals).unwrap().len(),
            1
        );
        let cursor_path = format!("/v1/intents/{intent_id}/connectors/{connector_id}/cursor");
        let (status, cursors) = request(&daemon, "GET", &cursor_path, "", None, None);
        assert_eq!(status, 200, "{cursors}");
        assert!(cursors.contains("linear-webhook"));
        let health_path = format!("/v1/intents/{intent_id}/connectors/{connector_id}/health");
        let (status, health) = request(&daemon, "GET", &health_path, "", None, None);
        assert_eq!(status, 200, "{health}");
        assert!(health.contains("healthy"));
        let (status, global_signals) = request(
            &daemon,
            "GET",
            "/v1/signals?provider=linear&cursor=0",
            "",
            None,
            None,
        );
        assert_eq!(status, 200, "{global_signals}");
        assert_eq!(
            serde_json::from_str::<Vec<Value>>(&global_signals)
                .unwrap()
                .len(),
            1
        );
        let (status, global_candidates) =
            request(&daemon, "GET", "/v1/candidates?cursor=0", "", None, None);
        assert_eq!(status, 200, "{global_candidates}");
        assert_eq!(
            serde_json::from_str::<Vec<Value>>(&global_candidates)
                .unwrap()
                .len(),
            1
        );
        let world_path = format!("/v1/intents/{intent_id}/world");
        let (status, world) = request(&daemon, "GET", &world_path, "", None, None);
        assert_eq!(status, 200, "{world}");
        assert_eq!(serde_json::from_str::<Vec<Value>>(&world).unwrap().len(), 1);
        let (status, brief) = request(&daemon, "GET", "/v1/world/brief", "", None, None);
        assert_eq!(status, 200, "{brief}");
        let brief: Value = serde_json::from_str(&brief).unwrap();
        assert_eq!(brief["waiting_external"].as_array().unwrap().len(), 1);
        let world_items: Vec<Value> = serde_json::from_str(&world).unwrap();
        let item_id = world_items[0]["id"].as_str().unwrap();
        let resolve_path = format!("/v1/intents/{intent_id}/world/resolve");
        let resolve_body = json!({
            "item_id": item_id,
            "kind": "completed",
            "note": "surface resolved"
        })
        .to_string();
        let (status, resolved) = request(
            &daemon,
            "POST",
            &resolve_path,
            &resolve_body,
            Some("world-resolve"),
            Some(revision),
        );
        assert_eq!(status, 200, "{resolved}");
        let (status, brief) = request(&daemon, "GET", "/v1/world/brief", "", None, None);
        assert_eq!(status, 200, "{brief}");
        let brief: Value = serde_json::from_str(&brief).unwrap();
        assert!(brief["waiting_external"].as_array().unwrap().is_empty());
        let projects_path = format!("/v1/intents/{intent_id}/projects");
        let (status, projects) = request(&daemon, "GET", &projects_path, "", None, None);
        assert_eq!(status, 200, "{projects}");
        let (status, projects) = request(&daemon, "GET", "/v1/projects", "", None, None);
        assert_eq!(status, 200, "{projects}");
        daemon.shutdown();
    }

    #[test]
    fn runtime_directory_and_token_are_private() {
        use std::os::unix::fs::PermissionsExt;

        let parent = tempfile::tempdir().unwrap();
        let dir = parent.path().join("runtime");
        std::fs::create_dir(&dir).unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        write_token(&dir, "first").unwrap();
        let token_path = dir.join("daemon-token");
        std::fs::set_permissions(&token_path, std::fs::Permissions::from_mode(0o644)).unwrap();

        write_token(&dir, "second").unwrap();
        assert_eq!(std::fs::read_to_string(&token_path).unwrap(), "second");
        assert_eq!(
            std::fs::metadata(&token_path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert!(std::fs::read_dir(&dir).unwrap().all(|entry| !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".daemon-token-")));

        let daemon = Daemon::start(DaemonConfig {
            dir: dir.clone(),
            bind: "127.0.0.1:0".into(),
            token: Some("daemon-token".into()),
        })
        .unwrap();
        assert_eq!(
            std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777,
            0o700
        );
        daemon.shutdown();
    }

    /// 读取 Intent 当前 Revision。
    ///
    /// 写操作都要求 `If-Match` 前置条件；测试里每一步重新读，
    /// 避免把"Revision 变化"和"路由行为"两件事混在一起断言。
    fn current_revision(daemon: &Daemon, intent_id: &str) -> u64 {
        let (status, body) = request(
            daemon,
            "GET",
            &format!("/v1/intents/{intent_id}"),
            "",
            None,
            None,
        );
        assert_eq!(status, 200, "{body}");
        serde_json::from_str::<Value>(&body).unwrap()["spec"]["revision"]
            .as_u64()
            .unwrap()
    }

    #[test]
    fn daemon_exposes_execution_plan_memory_and_scope_routes() {
        let dir = tempfile::tempdir().unwrap();
        let daemon = Daemon::start(DaemonConfig {
            dir: dir.path().to_path_buf(),
            bind: "127.0.0.1:0".into(),
            token: Some("test-token".into()),
        })
        .unwrap();

        let (status, body) = request(
            &daemon,
            "POST",
            "/v1/intents",
            r#"{"outcome":"daemon 路由覆盖","project":"test"}"#,
            Some("create-routes"),
            None,
        );
        assert_eq!(status, 200, "{body}");
        let snapshot: Value = serde_json::from_str(&body).unwrap();
        let intent_id = snapshot["spec"]["id"].as_str().unwrap().to_string();

        // Harness 清单：surface 用它解释执行器能力。
        let (status, harnesses) = request(&daemon, "GET", "/v1/harnesses", "", None, None);
        assert_eq!(status, 200, "{harnesses}");
        let harnesses: Value = serde_json::from_str(&harnesses).unwrap();
        let names: Vec<&str> = harnesses
            .as_array()
            .unwrap()
            .iter()
            .map(|value| value["name"].as_str().unwrap())
            .collect();
        assert!(names.contains(&"codex"), "{names:?}");
        assert!(names.contains(&"claude-code"), "{names:?}");

        // Scope 树：根 Scope 是委托树的起点。
        let scopes_path = format!("/v1/intents/{intent_id}/scopes");
        let (status, scopes) = request(&daemon, "GET", &scopes_path, "", None, None);
        assert_eq!(status, 200, "{scopes}");
        let scopes: Value = serde_json::from_str(&scopes).unwrap();
        assert_eq!(scopes.as_array().unwrap().len(), 1, "{scopes}");
        assert_eq!(scopes[0]["outcome"].as_str(), Some("daemon 路由覆盖"));

        // 委托子 Scope：权限只能低于或等于父级。
        let revision = current_revision(&daemon, &intent_id);
        let (status, child) = request(
            &daemon,
            "POST",
            &scopes_path,
            r#"{"task":"子任务","max_effect":"read_only"}"#,
            Some("spawn-1"),
            Some(revision),
        );
        assert_eq!(status, 200, "{child}");
        let child: Value = serde_json::from_str(&child).unwrap();
        assert!(child["scope_id"].as_str().is_some(), "{child}");

        // 推进一个 Kernel 步：长操作走 operation_id，不阻塞响应。
        let revision = current_revision(&daemon, &intent_id);
        let (status, advanced) = request(
            &daemon,
            "POST",
            &format!("/v1/intents/{intent_id}/advance"),
            r#"{"harness":"scripted"}"#,
            Some("advance-1"),
            Some(revision),
        );
        assert_eq!(status, 202, "{advanced}");
        let advanced: Value = serde_json::from_str(&advanced).unwrap();
        assert!(advanced["operation_id"].as_str().is_some(), "{advanced}");

        // 等待外部事件：停止空转执行。
        let revision = current_revision(&daemon, &intent_id);
        let (status, waiting) = request(
            &daemon,
            "POST",
            &format!("/v1/intents/{intent_id}/wait-external"),
            r#"{"reason":"等待合作方回复"}"#,
            Some("wait-1"),
            Some(revision),
        );
        assert_eq!(status, 200, "{waiting}");

        // 持久 Timer：重启后仍可用同一 ID 补齐唤醒。
        let revision = current_revision(&daemon, &intent_id);
        let (status, timer) = request(
            &daemon,
            "POST",
            &format!("/v1/intents/{intent_id}/timers"),
            r#"{"seconds":3600,"reason":"follow up"}"#,
            Some("timer-1"),
            Some(revision),
        );
        assert_eq!(status, 200, "{timer}");
        let timer: Value = serde_json::from_str(&timer).unwrap();
        assert!(timer["timer_id"].as_str().is_some(), "{timer}");

        // 计划 / 记忆 / 资源账：三个只读视图。
        let (status, plan) = request(
            &daemon,
            "GET",
            &format!("/v1/intents/{intent_id}/plan"),
            "",
            None,
            None,
        );
        assert_eq!(status, 200, "{plan}");
        let plan: Value = serde_json::from_str(&plan).unwrap();
        assert!(plan.get("history").is_some(), "{plan}");

        let (status, memory) = request(
            &daemon,
            "GET",
            &format!("/v1/intents/{intent_id}/memory"),
            "",
            None,
            None,
        );
        assert_eq!(status, 200, "{memory}");
        let memory: Value = serde_json::from_str(&memory).unwrap();
        assert!(memory.get("promoted").is_some(), "{memory}");
        assert!(memory.get("candidates").is_some(), "{memory}");

        let (status, resources) = request(
            &daemon,
            "GET",
            &format!("/v1/intents/{intent_id}/resources"),
            "",
            None,
            None,
        );
        assert_eq!(status, 200, "{resources}");
        let resources: Value = serde_json::from_str(&resources).unwrap();
        assert!(resources.get("budget").is_some(), "{resources}");
        assert!(resources.get("usage").is_some(), "{resources}");

        // Checkpoint 与 Compare：派生新 Scope 时不丢上下文。
        let revision = current_revision(&daemon, &intent_id);
        let (status, checkpoint) = request(
            &daemon,
            "POST",
            &format!("/v1/intents/{intent_id}/checkpoints"),
            "",
            Some("checkpoint-1"),
            Some(revision),
        );
        assert_eq!(status, 200, "{checkpoint}");
        let checkpoint: Value = serde_json::from_str(&checkpoint).unwrap();
        assert!(checkpoint["id"].as_str().is_some(), "{checkpoint}");

        let (status, comparison) = request(
            &daemon,
            "GET",
            &format!("/v1/intents/{intent_id}/compare"),
            "",
            None,
            None,
        );
        assert_eq!(status, 200, "{comparison}");

        // Opaque Profile：默认拒绝，必须显式放通。
        let revision = current_revision(&daemon, &intent_id);
        let (status, opaque) = request(
            &daemon,
            "POST",
            &format!("/v1/intents/{intent_id}/opaque"),
            r#"{"enabled":true,"allow_native_provider_network":false,"use_tool_gateway":true,"max_observed_effect":"reversible"}"#,
            Some("opaque-1"),
            Some(revision),
        );
        assert_eq!(status, 200, "{opaque}");
        let opaque: Value = serde_json::from_str(&opaque).unwrap();
        assert_eq!(
            opaque["opaque_harness_policy"]["enabled"],
            Value::Bool(true)
        );

        // 写操作缺 If-Match 时必须被拒绝：避免基于过期 Revision 写入。
        let (status, rejected) = request(
            &daemon,
            "POST",
            &format!("/v1/intents/{intent_id}/advance"),
            r#"{"harness":"scripted"}"#,
            Some("advance-no-revision"),
            None,
        );
        assert_eq!(status, 400, "{rejected}");

        daemon.shutdown();
    }

    #[test]
    fn daemon_exposes_the_contract_version_on_an_authenticated_endpoint() {
        let dir = tempfile::tempdir().unwrap();
        let daemon = Daemon::start(DaemonConfig {
            dir: dir.path().to_path_buf(),
            bind: "127.0.0.1:0".into(),
            token: Some("test-token".into()),
        })
        .unwrap();

        let (status, body) = request(&daemon, "GET", "/v1/version", "", None, None);
        assert_eq!(status, 200, "{body}");
        let version: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(
            version["contract_version"].as_str(),
            Some(else_protocol::api::CONTRACT_VERSION)
        );
        assert_eq!(
            version["protocol_version"].as_str(),
            Some(else_protocol::PROTOCOL_VERSION)
        );

        daemon.shutdown();
    }

    #[test]
    fn daemon_does_not_write_a_token_when_it_cannot_bind() {
        // 端口被占时不能把令牌写进 runtime 目录 ——
        // 那份令牌对谁都不成立，却会让下一次连接拿着错令牌来敲门。
        let first_dir = tempfile::tempdir().unwrap();
        let first = Daemon::start(DaemonConfig {
            dir: first_dir.path().to_path_buf(),
            bind: "127.0.0.1:0".into(),
            token: Some("first-token".into()),
        })
        .unwrap();
        let bind = first.url.trim_start_matches("http://").to_string();

        let second_dir = tempfile::tempdir().unwrap();
        let second = Daemon::start(DaemonConfig {
            dir: second_dir.path().to_path_buf(),
            bind: bind.clone(),
            token: Some("second-token".into()),
        });
        assert!(matches!(second, Err(DaemonError::Bind(_))));
        assert!(
            !second_dir.path().join("daemon-token").exists(),
            "绑定失败时不应写出令牌文件"
        );

        first.shutdown();
    }

    #[test]
    fn daemon_rejects_stale_revision_precondition() {
        let dir = tempfile::tempdir().unwrap();
        let daemon = Daemon::start(DaemonConfig {
            dir: dir.path().to_path_buf(),
            bind: "127.0.0.1:0".into(),
            token: Some("test-token".into()),
        })
        .unwrap();

        let (status, body) = request(
            &daemon,
            "POST",
            "/v1/intents",
            r#"{"outcome":"revision 冲突"}"#,
            Some("create-rev"),
            None,
        );
        assert_eq!(status, 200, "{body}");
        let snapshot: Value = serde_json::from_str(&body).unwrap();
        let intent_id = snapshot["spec"]["id"].as_str().unwrap().to_string();

        let (status, rejected) = request(
            &daemon,
            "POST",
            &format!("/v1/intents/{intent_id}/advance"),
            r#"{"harness":"scripted"}"#,
            Some("advance-stale"),
            Some(9999),
        );
        assert_eq!(status, 409, "{rejected}");

        daemon.shutdown();
    }
}
