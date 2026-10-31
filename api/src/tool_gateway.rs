//! Loopback Tool Gateway：Opaque Harness 通过 Bearer Token 请求受控工具。

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use else_gateway::{ExecContext, GatewayError, ToolGateway};
use else_ledger::Ledger;
use else_protocol::effect::{SideEffectKind, ToolCall};
use else_protocol::ids::ToolCallId;

use crate::Result;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ToolGatewayEndpoint {
    pub host: String,
    pub port: u16,
    pub token: String,
}

pub(crate) struct ToolGatewayServer {
    endpoint: ToolGatewayEndpoint,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl ToolGatewayServer {
    pub(crate) fn start(ctx: ExecContext, db_path: PathBuf, workspace: PathBuf) -> Result<Self> {
        let listener = TcpListener::bind(("127.0.0.1", 0))?;
        listener.set_nonblocking(true)?;
        let port = listener.local_addr()?.port();
        let token = uuid::Uuid::new_v4().to_string();
        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = stop.clone();
        let serial = Arc::new(Mutex::new(()));
        let worker_token = token.clone();
        let worker = thread::spawn(move || {
            while !thread_stop.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        let ctx = ctx.clone();
                        let db_path = db_path.clone();
                        let workspace = workspace.clone();
                        let token = worker_token.clone();
                        let serial = serial.clone();
                        thread::spawn(move || {
                            let _guard = serial.lock().unwrap_or_else(|poison| poison.into_inner());
                            handle_request(stream, &ctx, &db_path, &workspace, &token);
                        });
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(10));
                    }
                    Err(_) => break,
                }
            }
        });
        Ok(Self {
            endpoint: ToolGatewayEndpoint {
                host: "127.0.0.1".into(),
                port,
                token,
            },
            stop,
            worker: Some(worker),
        })
    }

    pub(crate) fn endpoint(&self) -> &ToolGatewayEndpoint {
        &self.endpoint
    }
}

impl Drop for ToolGatewayServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = TcpStream::connect_timeout(
            &SocketAddr::from(([127, 0, 0, 1], self.endpoint.port)),
            Duration::from_millis(100),
        );
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn handle_request(
    mut stream: TcpStream,
    ctx: &ExecContext,
    db_path: &Path,
    workspace: &Path,
    token: &str,
) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(30)));
    let Some((headers, body)) = read_http_request(&mut stream) else {
        respond(&mut stream, 400, &json_error("malformed request"));
        return;
    };
    if !headers
        .lines()
        .any(|line| line.eq_ignore_ascii_case(&format!("Authorization: Bearer {token}")))
    {
        respond(&mut stream, 401, &json_error("invalid tool gateway token"));
        return;
    }
    let Ok(request) = serde_json::from_slice::<serde_json::Value>(&body) else {
        respond(&mut stream, 400, &json_error("invalid JSON body"));
        return;
    };
    let Some(tool) = request["tool"].as_str() else {
        respond(&mut stream, 400, &json_error("missing tool"));
        return;
    };
    let Some(effect) = tool_effect(tool) else {
        respond(&mut stream, 400, &json_error("unknown tool"));
        return;
    };
    let idempotency_key = request["idempotency_key"]
        .as_str()
        .map(str::to_string)
        .unwrap_or_else(|| format!("gateway-{}", uuid::Uuid::new_v4()));
    let call = ToolCall {
        id: ToolCallId::new(),
        tool: tool.into(),
        effect,
        input: request["input"].clone(),
        idempotency_key,
    };
    let ledger = match Ledger::open(db_path) {
        Ok(ledger) => ledger,
        Err(error) => {
            respond(&mut stream, 500, &json_error(&error.to_string()));
            return;
        }
    };
    let tools = ToolGateway::new(&ledger, workspace.to_path_buf());
    match tools.execute(ctx, call) {
        Ok(outcome) => respond(
            &mut stream,
            200,
            &serde_json::json!({ "ok": true, "outcome": outcome }).to_string(),
        ),
        Err(GatewayError::NeedsApproval(request_id)) => respond(
            &mut stream,
            409,
            &serde_json::json!({
                "ok": false,
                "error": "needs_approval",
                "request_id": request_id,
            })
            .to_string(),
        ),
        Err(error) => respond(
            &mut stream,
            403,
            &serde_json::json!({ "ok": false, "error": error.to_string() }).to_string(),
        ),
    }
}

fn tool_effect(tool: &str) -> Option<SideEffectKind> {
    match tool {
        "fs.read" => Some(SideEffectKind::ReadOnly),
        "fs.write" => Some(SideEffectKind::Reversible),
        "shell" => Some(SideEffectKind::Unknown),
        _ => None,
    }
}

fn read_http_request(stream: &mut TcpStream) -> Option<(String, Vec<u8>)> {
    let mut buffer = Vec::new();
    let mut chunk = [0_u8; 1024];
    while !buffer.windows(4).any(|window| window == b"\r\n\r\n") {
        let read = stream.read(&mut chunk).ok()?;
        if read == 0 || buffer.len() >= 64 * 1024 {
            return None;
        }
        buffer.extend_from_slice(&chunk[..read]);
    }
    let header_end = buffer.windows(4).position(|window| window == b"\r\n\r\n")? + 4;
    let headers = String::from_utf8(buffer[..header_end].to_vec()).ok()?;
    let content_length = headers.lines().find_map(|line| {
        line.strip_prefix("Content-Length:")
            .and_then(|value| value.trim().parse::<usize>().ok())
    })?;
    while buffer.len() < header_end + content_length {
        let read = stream.read(&mut chunk).ok()?;
        if read == 0 {
            return None;
        }
        buffer.extend_from_slice(&chunk[..read]);
    }
    Some((
        headers,
        buffer[header_end..header_end + content_length].to_vec(),
    ))
}

fn respond(stream: &mut TcpStream, status: u16, body: &str) {
    let reason = match status {
        200 => "OK",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        409 => "Conflict",
        _ => "Internal Server Error",
    };
    let _ = write!(
        stream,
        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
}

fn json_error(message: &str) -> String {
    serde_json::json!({ "ok": false, "error": message }).to_string()
}
