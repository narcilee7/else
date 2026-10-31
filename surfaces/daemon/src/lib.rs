//! Loopback HTTP surface for the resident Intent Runtime.

use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use chrono::Utc;
use else_api::{ApiError, RuntimeHandle, RuntimeService};
use else_ledger::{IntentProjection, StoredEvent};
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
        write_token(&config.dir, &token)?;

        let service = RuntimeService::start(&config.dir)?;
        let handle = service.handle();
        let server =
            Server::http(&config.bind).map_err(|error| DaemonError::Bind(error.to_string()))?;
        let addr = server
            .server_addr()
            .to_ip()
            .ok_or_else(|| DaemonError::Bind("daemon requires a TCP listener".into()))?;
        let url = format!("http://{addr}");
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
}

#[derive(Serialize)]
struct IntentSnapshot {
    spec: IntentSpec,
    state: IntentState,
    decisions: Vec<else_protocol::attention::AttentionRequest>,
    artifacts: Vec<else_protocol::artifact::Artifact>,
    resource_budget: else_protocol::resource::ResourceBudget,
    resource_usage: else_protocol::resource::ResourceUsage,
    cursor: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    operation_id: Option<String>,
}

#[derive(Serialize)]
struct EventResponse {
    seq: u64,
    event_id: String,
    intent_id: String,
    scope_id: String,
    run_id: Option<String>,
    generation: u64,
    intent_revision: u64,
    occurred_at: String,
    payload: Value,
}

fn handle_http(mut request: Request, state: &AppState) {
    let path = request.url().split('?').next().unwrap_or("/").to_string();
    if path == "/v1/health" {
        respond_json(request, 200, &json!({ "ok": true, "version": "0.1.0" }));
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

    let segments: Vec<_> = path.trim_matches('/').split('/').collect();
    let result = route(&request, state, &segments, &body);
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
                DaemonError::Api(ApiError::Conflict(_)) => 409,
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
    if segments.len() >= 3 && segments[0..2] == ["v1", "intents"] {
        let intent_id = else_protocol::ids::IntentId::from(segments[2]);
        if segments.len() == 3 && request.method() == &Method::Get {
            return Ok((
                200,
                json_string(&snapshot(&state.handle.projection(&intent_id)?))?,
            ));
        }
        if segments.len() == 4 && segments[3] == "run" && request.method() == &Method::Post {
            check_expected_revision(request, &state.handle.projection(&intent_id)?)?;
            let mut current = snapshot(&state.handle.projection(&intent_id)?);
            current.operation_id = Some(spawn_operation(state, "run", &intent_id, {
                let handle = state.handle.clone();
                let intent_id = intent_id.clone();
                move || handle.drive(&intent_id, None, 16).map(|_| ())
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
                move || handle.decide(&intent_id, &decision_id, choice).map(|_| ())
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
    operation: impl FnOnce() -> else_api::Result<()> + Send + 'static,
) -> String {
    let id = uuid::Uuid::new_v4().to_string();
    let record = Operation {
        id: id.clone(),
        intent_id: intent_id.to_string(),
        kind: kind.into(),
        status: "running".into(),
        error: None,
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
                Ok(()) => record.status = "completed".into(),
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
        (name == key).then(|| value.to_string())
    })
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

fn write_token(dir: &Path, token: &str) -> Result<()> {
    let path = dir.join("daemon-token");
    let mut options = OpenOptions::new();
    options.create(true).write(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    use std::io::Write;
    file.write_all(token.as_bytes())?;
    Ok(())
}

pub fn run_from_args() -> Result<()> {
    let mut dir = PathBuf::from(".else");
    let mut bind = "127.0.0.1:4180".to_string();
    let mut token = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--dir" => dir = PathBuf::from(args.next().unwrap_or_else(|| ".else".into())),
            "--bind" => bind = args.next().unwrap_or(bind),
            "--token" => token = args.next(),
            "--help" | "-h" => {
                println!("else-daemon [--dir PATH] [--bind 127.0.0.1:4180] [--token TOKEN]");
                return Ok(());
            }
            _ => return Err(DaemonError::InvalidRequest(format!("unknown arg {arg}"))),
        }
    }
    let daemon = Daemon::start(DaemonConfig { dir, bind, token })?;
    println!("runtime: {}", daemon.url);
    println!("token: {}", daemon.token);
    loop {
        thread::park_timeout(Duration::from_secs(60));
    }
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
}
