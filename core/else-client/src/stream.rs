//! Runtime 事件流客户端（Server-Sent Events）。
//!
//! TUI 用它做增量刷新：只重取变化的部分，不必轮询整份快照。
//! 流由 daemon 侧在 60 秒后关闭，客户端用最新的 `cursor` 重连即可。

use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;
use std::time::Duration;

use else_protocol::api::EventResponse;

use crate::{ClientError, Result};

/// 一个已连接的事件流。
pub struct EventStream {
    reader: BufReader<TcpStream>,
    base: String,
    token: String,
    cursor: u64,
    /// 尚未解析完的一行。
    pending: Option<String>,
}

impl EventStream {
    /// 连接到 Runtime 的事件流。
    pub fn connect(base: &str, token: &str, cursor: u64) -> Result<Self> {
        let authority = base
            .trim_start_matches("http://")
            .trim_start_matches("https://")
            .trim_end_matches('/');
        let address = authority
            .rsplit_once(':')
            .map(|(host, port)| format!("{host}:{port}"))
            .unwrap_or_else(|| format!("{authority}:80"));
        let mut stream = TcpStream::connect(&address)
            .map_err(|error| ClientError::Unreachable(format!("{address}: {error}")))?;
        stream
            .set_read_timeout(Some(Duration::from_secs(90)))
            .map_err(|error| ClientError::Unreachable(error.to_string()))?;
        let request = format!(
            "GET /v1/events/stream?cursor={cursor} HTTP/1.1\r\nHost: {authority}\r\n\
             Authorization: Bearer {token}\r\nAccept: text/event-stream\r\n\
             Connection: keep-alive\r\n\r\n"
        );
        stream
            .write_all(request.as_bytes())
            .map_err(|error| ClientError::Unreachable(error.to_string()))?;
        let mut reader = BufReader::new(stream);
        // 跳过 HTTP 响应头，直到空行。
        let mut line = String::new();
        loop {
            line.clear();
            let read = reader
                .read_line(&mut line)
                .map_err(|error| ClientError::Unreachable(error.to_string()))?;
            if read == 0 {
                return Err(ClientError::Unreachable("event stream closed".into()));
            }
            if line.trim().is_empty() {
                break;
            }
        }
        Ok(Self {
            reader,
            base: base.trim_end_matches('/').to_string(),
            token: token.to_string(),
            cursor,
            pending: None,
        })
    }

    /// 当前游标：客户端用它记录「读到哪了」。
    pub fn cursor(&self) -> u64 {
        self.cursor
    }

    /// 读下一个事件。流结束时返回 `Ok(None)`。
    pub fn next_event(&mut self) -> Result<Option<EventResponse>> {
        loop {
            let line = match self.pending.take() {
                Some(line) => line,
                None => {
                    let mut line = String::new();
                    let read = self
                        .reader
                        .read_line(&mut line)
                        .map_err(|error| ClientError::Unreachable(error.to_string()))?;
                    if read == 0 {
                        return Ok(None);
                    }
                    line
                }
            };
            let trimmed = line.trim_end_matches(['\r', '\n']);
            if trimmed.is_empty() {
                continue;
            }
            let Some(payload) = trimmed.strip_prefix("data:") else {
                continue;
            };
            let payload = payload.trim();
            if payload.is_empty() {
                continue;
            }
            match serde_json::from_str::<EventResponse>(payload) {
                Ok(event) => {
                    self.cursor = event.seq;
                    return Ok(Some(event));
                }
                Err(error) => {
                    return Err(ClientError::Malformed(format!(
                        "event stream payload: {error}"
                    )))
                }
            }
        }
    }

    /// 断开并用最新游标重连。
    pub fn reconnect(&mut self) -> Result<()> {
        let cursor = self.cursor;
        let fresh = EventStream::connect(&self.base, &self.token, cursor)?;
        *self = fresh;
        Ok(())
    }
}
