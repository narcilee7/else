//! 持久 MCP stdio session：initialize 一次，request 多次。

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout};
use std::sync::{Mutex, OnceLock};

use serde_json::{json, Value};

static SESSIONS: OnceLock<Mutex<HashMap<(PathBuf, String), McpSession>>> = OnceLock::new();

struct McpSession {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    next_id: u64,
}

impl McpSession {
    fn start(workspace: &Path, command: &str) -> Result<Self, String> {
        let sandbox = else_environment::Sandbox::new(
            else_environment::SandboxProfile::new(workspace)
                .with_network(else_environment::NetworkPolicy::Deny),
        )
        .map_err(|error| error.to_string())?;
        let mut child = sandbox
            .spawn_shell_interactive(command, &[])
            .map_err(|error| error.to_string())?;
        let stdin = child.stdin.take().ok_or("MCP stdin unavailable")?;
        let stdout = child.stdout.take().ok_or("MCP stdout unavailable")?;
        if let Some(stderr) = child.stderr.take() {
            std::thread::spawn(move || {
                let mut reader = BufReader::new(stderr);
                let mut line = String::new();
                while reader.read_line(&mut line).is_ok_and(|read| read > 0) {
                    line.clear();
                }
            });
        }
        let mut session = Self {
            child,
            stdin,
            stdout: BufReader::new(stdout),
            next_id: 1,
        };
        session
            .request(
                "initialize",
                json!({
                    "protocolVersion":"2025-03-26",
                    "capabilities":{},
                    "clientInfo":{"name":"else","version":"0.1.0"}
                }),
            )
            .map_err(|error| format!("MCP initialize: {error}"))?;
        writeln!(
            session.stdin,
            "{}",
            json!({"jsonrpc":"2.0","method":"notifications/initialized"})
        )
        .map_err(|error| error.to_string())?;
        Ok(session)
    }

    fn request(&mut self, method: &str, params: Value) -> Result<Value, String> {
        let id = self.next_id;
        self.next_id = self.next_id.saturating_add(1);
        writeln!(
            self.stdin,
            "{}",
            json!({"jsonrpc":"2.0","id":id,"method":method,"params":params})
        )
        .map_err(|error| error.to_string())?;
        self.stdin.flush().map_err(|error| error.to_string())?;
        let mut line = String::new();
        loop {
            line.clear();
            let read = self
                .stdout
                .read_line(&mut line)
                .map_err(|error| error.to_string())?;
            if read == 0 {
                return Err("MCP server exited".into());
            }
            let Ok(value) = serde_json::from_str::<Value>(&line) else {
                continue;
            };
            if value["id"] == id {
                if value.get("error").is_some() {
                    return Err(value["error"].to_string());
                }
                return Ok(value["result"].clone());
            }
        }
    }
}

impl Drop for McpSession {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

pub fn list_tools(workspace: &Path, command: &str) -> Result<Value, String> {
    with_session(workspace, command, |session| {
        session.request("tools/list", json!({}))
    })
}

pub fn call_tool(
    workspace: &Path,
    command: &str,
    tool_name: &str,
    arguments: Value,
) -> Result<Value, String> {
    with_session(workspace, command, |session| {
        session.request(
            "tools/call",
            json!({"name":tool_name,"arguments":arguments}),
        )
    })
}

fn with_session<T>(
    workspace: &Path,
    command: &str,
    mut action: impl FnMut(&mut McpSession) -> Result<T, String>,
) -> Result<T, String> {
    let sessions = SESSIONS.get_or_init(|| Mutex::new(HashMap::new()));
    let key = (workspace.to_path_buf(), command.to_string());
    let mut sessions = sessions.lock().unwrap_or_else(|poison| poison.into_inner());
    if !sessions.contains_key(&key) {
        sessions.insert(key.clone(), McpSession::start(workspace, command)?);
    }
    let first = action(sessions.get_mut(&key).expect("session inserted"));
    if first.is_ok() {
        return first;
    }
    sessions.remove(&key);
    sessions.insert(key.clone(), McpSession::start(workspace, command)?);
    action(sessions.get_mut(&key).expect("session replaced"))
}
