//! 本地 HTTPS CONNECT 代理：Harness 只能访问显式域名 Allowlist。

use std::io::{Read, Write};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use else_protocol::egress::EgressDecision;

const MAX_HEADER_BYTES: usize = 16 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EgressProxyEndpoint {
    pub host: String,
    pub port: u16,
}

pub struct EgressProxy {
    endpoint: EgressProxyEndpoint,
    decisions: Arc<Mutex<Vec<EgressDecision>>>,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl EgressProxy {
    pub fn start(allowlist: Vec<String>) -> std::io::Result<Self> {
        let listener = TcpListener::bind(("127.0.0.1", 0))?;
        listener.set_nonblocking(true)?;
        let port = listener.local_addr()?.port();
        let decisions = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let thread_decisions = decisions.clone();
        let thread_stop = stop.clone();
        let allowlist = Arc::new(
            allowlist
                .into_iter()
                .map(|host| host.trim().trim_end_matches('.').to_lowercase())
                .collect::<Vec<_>>(),
        );
        let worker = thread::spawn(move || {
            while !thread_stop.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        let allowlist = allowlist.clone();
                        let decisions = thread_decisions.clone();
                        thread::spawn(move || handle_connection(stream, &allowlist, &decisions));
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(10));
                    }
                    Err(_) => break,
                }
            }
        });
        Ok(Self {
            endpoint: EgressProxyEndpoint {
                host: "127.0.0.1".into(),
                port,
            },
            decisions,
            stop,
            worker: Some(worker),
        })
    }

    pub fn endpoint(&self) -> &EgressProxyEndpoint {
        &self.endpoint
    }

    pub fn decisions(&self) -> Vec<EgressDecision> {
        self.decisions
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .clone()
    }
}

impl Drop for EgressProxy {
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

fn handle_connection(
    mut client: TcpStream,
    allowlist: &[String],
    decisions: &Mutex<Vec<EgressDecision>>,
) {
    let _ = client.set_read_timeout(Some(Duration::from_secs(10)));
    let _ = client.set_write_timeout(Some(Duration::from_secs(10)));
    let Some(header) = read_header(&mut client) else {
        return;
    };
    let first_line = header.lines().next().unwrap_or_default();
    let Some((host, port)) = parse_connect_target(first_line) else {
        let _ = client.write_all(
            b"HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        );
        return;
    };
    let allowed = host_allowed(&host, allowlist);
    decisions
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
        .push(EgressDecision {
            host: host.clone(),
            allowed,
            reason: if allowed {
                "matched Opaque Harness provider allowlist".into()
            } else {
                "host is absent from Opaque Harness provider allowlist".into()
            },
        });
    if !allowed {
        let _ = client
            .write_all(b"HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
        return;
    }
    let Ok(upstream) = TcpStream::connect((host.as_str(), port)) else {
        let _ = client.write_all(
            b"HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        );
        return;
    };
    let _ = upstream.set_read_timeout(Some(Duration::from_secs(60)));
    let _ = upstream.set_write_timeout(Some(Duration::from_secs(60)));
    if client
        .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
        .is_err()
    {
        return;
    }
    tunnel(client, upstream);
}

fn read_header(stream: &mut TcpStream) -> Option<String> {
    let mut buffer = Vec::new();
    let mut chunk = [0_u8; 1024];
    while buffer.len() < MAX_HEADER_BYTES {
        let read = stream.read(&mut chunk).ok()?;
        if read == 0 {
            return None;
        }
        buffer.extend_from_slice(&chunk[..read]);
        if buffer.windows(4).any(|window| window == b"\r\n\r\n") {
            return String::from_utf8(buffer).ok();
        }
    }
    None
}

fn parse_connect_target(request_line: &str) -> Option<(String, u16)> {
    let mut parts = request_line.split_whitespace();
    if parts.next()? != "CONNECT" {
        return None;
    }
    let authority = parts.next()?;
    let (host, port) = authority.rsplit_once(':')?;
    Some((
        host.trim_matches(['[', ']']).to_lowercase(),
        port.parse().ok()?,
    ))
}

fn host_allowed(host: &str, allowlist: &[String]) -> bool {
    let host = host.to_lowercase();
    allowlist.iter().any(|allowed| {
        if let Some(suffix) = allowed.strip_prefix('.') {
            host == suffix || host.ends_with(&format!(".{suffix}"))
        } else {
            host == *allowed
        }
    })
}

fn tunnel(mut client: TcpStream, mut upstream: TcpStream) {
    let Ok(mut client_reader) = client.try_clone() else {
        return;
    };
    let Ok(mut upstream_writer) = upstream.try_clone() else {
        return;
    };
    let upload = thread::spawn(move || {
        let _ = std::io::copy(&mut client_reader, &mut upstream_writer);
        let _ = upstream_writer.shutdown(Shutdown::Write);
    });
    let _ = std::io::copy(&mut upstream, &mut client);
    let _ = client.shutdown(Shutdown::Write);
    let _ = upload.join();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn domain_allowlist_supports_exact_and_suffix_hosts() {
        let allowlist = vec!["api.openai.com".into(), ".anthropic.com".into()];
        assert!(host_allowed("api.openai.com", &allowlist));
        assert!(host_allowed("api.anthropic.com", &allowlist));
        assert!(!host_allowed("example.com", &allowlist));
        assert!(!host_allowed("api.openai.com.evil.test", &allowlist));
    }

    #[test]
    fn proxy_blocks_disallowed_connect_and_audits_decision() {
        let proxy = EgressProxy::start(vec!["api.openai.com".into()]).unwrap();
        let mut stream =
            TcpStream::connect((proxy.endpoint().host.as_str(), proxy.endpoint().port)).unwrap();
        stream
            .write_all(b"CONNECT example.com:443 HTTP/1.1\r\nHost: example.com\r\n\r\n")
            .unwrap();
        let mut response = [0_u8; 32];
        let read = stream.read(&mut response).unwrap();
        assert!(String::from_utf8_lossy(&response[..read]).contains("403"));
        let decisions = proxy.decisions();
        assert_eq!(decisions.len(), 1);
        assert!(!decisions[0].allowed);
    }
}
