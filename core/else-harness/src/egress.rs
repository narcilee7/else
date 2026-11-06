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
    // 监听器是非阻塞的，macOS 上 accept 出来的连接会继承这个标志；
    // 不显式改回阻塞，隧道里的第一次 read 就拿到 EAGAIN，隧道当场断掉。
    let _ = client.set_nonblocking(false);
    let _ = client.set_read_timeout(Some(Duration::from_secs(10)));
    let _ = client.set_write_timeout(Some(Duration::from_secs(10)));
    // `read_header` 会把同一次 read 里多出来的字节一并交回来：
    // 客户端经常把 CONNECT 与 TLS ClientHello 连在一起发，
    // 丢掉那几个字节，握手就废了，对端看到的就是连接被重置。
    let Some((header, mut leftover)) = read_header(&mut client) else {
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
    let Ok(mut upstream) = TcpStream::connect((host.as_str(), port)) else {
        let _ = client.write_all(
            b"HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        );
        return;
    };
    // 读 CONNECT 头用的 10s 超时到此为止。隧道里两侧都会长时间安静
    // （模型正在思考），超时会把 `io::copy` 变成“写端半关”，对端看到的就是
    // 连接被重置。隧道阶段不设任何读超时。
    let _ = client.set_read_timeout(None);
    let _ = client.set_write_timeout(None);
    let _ = upstream.set_read_timeout(None);
    let _ = upstream.set_write_timeout(None);
    if client
        .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
        .is_err()
    {
        return;
    }
    if !leftover.is_empty() && upstream.write_all(&leftover).is_err() {
        return;
    }
    leftover.clear();
    tunnel(client, upstream);
}

/// 读到 `\r\n\r\n` 为止，返回 （头部文本, 头部之后多出来的字节）。
///
/// 多出来的字节必须交给调用方转给上游：CONNECT 与 TLS 握手经常在同一个
/// TCP 段里到达，把它们当成“头部的一部分”丢掉是最隐蔽的隧道 bug。
fn read_header(stream: &mut TcpStream) -> Option<(String, Vec<u8>)> {
    let mut buffer = Vec::new();
    let mut chunk = [0_u8; 1024];
    while buffer.len() < MAX_HEADER_BYTES {
        let read = stream.read(&mut chunk).ok()?;
        if read == 0 {
            return None;
        }
        buffer.extend_from_slice(&chunk[..read]);
        if let Some(position) = buffer.windows(4).position(|window| window == b"\r\n\r\n") {
            let end = position + 4;
            let header = String::from_utf8(buffer[..end].to_vec()).ok()?;
            return Some((header, buffer[end..].to_vec()));
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

    /// 隧道必须真的能双向转发字节。
    ///
    /// 回归的是两个真实缺陷：accept 出来的连接继承了监听器的非阻塞标志
    /// （第一次 read 就是 EAGAIN，隧道当场断），以及头部后多出来的字节
    /// （TLS ClientHello 常与 CONNECT 同时到达）被当成头部丢掉。
    #[test]
    fn tunnel_relays_bytes_after_the_connect_header() {
        use std::net::TcpListener;

        let upstream = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let upstream_port = upstream.local_addr().unwrap().port();
        let echo = std::thread::spawn(move || {
            let (mut stream, _) = upstream.accept().unwrap();
            let mut buffer = [0_u8; 64];
            let read = stream.read(&mut buffer).unwrap();
            stream.write_all(&buffer[..read]).unwrap();
        });

        let proxy = EgressProxy::start(vec!["127.0.0.1".into()]).unwrap();
        let mut client =
            TcpStream::connect((proxy.endpoint().host.as_str(), proxy.endpoint().port)).unwrap();
        // CONNECT 与随后的负载在同一次写入里：这正是丢字节路径。
        client
            .write_all(
                format!(
                    "CONNECT 127.0.0.1:{upstream_port} HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\nping"
                )
                .as_bytes(),
            )
            .unwrap();

        let mut header = [0_u8; 64];
        let read = client.read(&mut header).unwrap();
        assert!(String::from_utf8_lossy(&header[..read]).contains("200"));

        let mut echoed = [0_u8; 8];
        let read = client.read(&mut echoed).unwrap();
        assert_eq!(&echoed[..read], b"ping");
        let _ = echo.join();
    }
}
