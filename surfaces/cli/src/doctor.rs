//! `else-bot doctor`：一次说清「现在这台机器上的 else 是什么状态」。
//!
//! 出问题时，用户不该去翻日志猜。把该看的都看一遍，然后给出一份
//! **可以直接贴给别人的**报告。

use std::path::{Path, PathBuf};

use else_client::RuntimeClient;
use serde::Serialize;

use crate::service;

/// 一项检查。
#[derive(Debug, Clone, Serialize)]
pub struct Check {
    pub name: String,
    pub ok: bool,
    /// 人话说明。
    pub detail: String,
    /// 失败时的建议动作。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
}

/// 诊断报告。
#[derive(Debug, Clone, Serialize)]
pub struct DoctorReport {
    pub runtime_dir: PathBuf,
    pub runtime_url: String,
    pub checks: Vec<Check>,
}

impl DoctorReport {
    /// 是否全部通过。
    pub fn healthy(&self) -> bool {
        self.checks.iter().all(|check| check.ok)
    }

    /// 打印人类可读的报告。
    pub fn print(&self) {
        println!("else-bot doctor");
        println!("  runtime: {}", self.runtime_dir.display());
        println!("  url:     {}", self.runtime_url);
        println!();
        for check in &self.checks {
            let mark = if check.ok { "ok  " } else { "FAIL" };
            println!("[{mark}] {}: {}", check.name, check.detail);
            if let Some(hint) = &check.hint {
                if !check.ok {
                    println!("       → {hint}");
                }
            }
        }
        println!();
        if self.healthy() {
            println!("all good.");
        } else {
            println!("some checks failed — the hints above say what to do.");
        }
    }
}

/// 跑一遍诊断。
pub fn run(client: &RuntimeClient, runtime_dir: &Path) -> DoctorReport {
    let mut checks = Vec::new();

    // 1) Runtime 目录。
    checks.push(check_runtime_dir(runtime_dir));

    // 2) 后台服务。
    checks.push(check_service(runtime_dir));

    // 3) 当前会话接的到底是常驻 Runtime 还是临时起的一个。
    checks.push(check_resident(client, runtime_dir));

    // 4) Runtime 可达性与契约版本。
    match client.version() {
        Ok(version) => checks.push(Check {
            name: "runtime".into(),
            ok: true,
            detail: format!(
                "reachable at {} (contract {}, protocol {})",
                client.base_url(),
                version.contract_version,
                version.protocol_version
            ),
            hint: None,
        }),
        Err(error) => checks.push(Check {
            name: "runtime".into(),
            ok: false,
            detail: format!("cannot reach the runtime: {error}"),
            hint: Some(
                "运行 `else-bot daemon install` 让 Runtime 常驻，或用 `else-bot serve` 前台启动。"
                    .into(),
            ),
        }),
    }

    // 4) 令牌有效性 —— 与「可达」是两件事。
    match client.probe() {
        Ok(()) => checks.push(Check {
            name: "token".into(),
            ok: true,
            detail: "the token belongs to this runtime".into(),
            hint: None,
        }),
        Err(error) => checks.push(Check {
            name: "token".into(),
            ok: false,
            detail: format!("the token was rejected: {error}"),
            hint: Some("删除 runtime 目录里的 daemon-token 后重启 Runtime。".into()),
        }),
    }

    // 5) Ledger 可写。
    checks.push(check_ledger_writable(runtime_dir));

    // 6) Connector 健康。
    checks.push(check_connectors(client));

    DoctorReport {
        runtime_dir: runtime_dir.to_path_buf(),
        runtime_url: client.base_url().to_string(),
        checks,
    }
}

/// Runtime 目录检查。
fn check_runtime_dir(runtime_dir: &Path) -> Check {
    if !runtime_dir.exists() {
        return Check {
            name: "runtime directory".into(),
            ok: false,
            detail: format!("{} does not exist", runtime_dir.display()),
            hint: Some("运行 `else-bot onboard` 创建它。".into()),
        };
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(runtime_dir)
            .map(|metadata| metadata.permissions().mode() & 0o777)
            .unwrap_or_default();
        if mode & 0o077 != 0 {
            return Check {
                name: "runtime directory".into(),
                ok: false,
                detail: format!(
                    "{} is readable by group or others (mode {mode:o})",
                    runtime_dir.display()
                ),
                hint: Some(format!("chmod 700 {}", runtime_dir.display())),
            };
        }
    }
    Check {
        name: "runtime directory".into(),
        ok: true,
        detail: format!("{} is private", runtime_dir.display()),
        hint: None,
    }
}

/// 后台服务检查。
///
/// 三个分开的事实：装没装、加载没有、进程在不在。只看 label 会把一个
/// 崩溃循环的服务当成健康。
fn check_service(runtime_dir: &Path) -> Check {
    let status = service::status(runtime_dir);
    let mut problems = Vec::new();
    if !status.running {
        problems.push(status.detail.clone());
    }
    if let Some(unit_dir) = status.unit_runtime_dir.as_ref() {
        if unit_dir != runtime_dir {
            problems.push(format!(
                "服务定义指向 {}，与当前 runtime 目录不一致",
                unit_dir.display()
            ));
        }
    }
    if problems.is_empty() {
        return Check {
            name: "background service".into(),
            ok: true,
            detail: status.detail,
            hint: None,
        };
    }
    Check {
        name: "background service".into(),
        ok: false,
        detail: problems.join("；"),
        hint: Some(
            "`else-bot daemon install` 会重新注册；日志见 ~/Library/Logs/else-runtime.log。".into(),
        ),
    }
}

/// 当前会话接的是常驻 Runtime 还是命令自己起的临时 Runtime。
///
/// 常驻 daemon 启动时会写 `daemon.pid`（pid + 监听地址）。没有它，或者地址
/// 不匹配，就说明这条命令连的是一个用完即弃的替身 —— 连接器同步、定时器与
/// 通知都不会发生，而用户看到的仍是"一切正常"。
fn check_resident(client: &RuntimeClient, runtime_dir: &Path) -> Check {
    let path = runtime_dir.join("daemon.pid");
    let Ok(body) = std::fs::read_to_string(&path) else {
        return Check {
            name: "resident runtime".into(),
            ok: false,
            detail: format!(
                "没有常驻 Runtime；这条命令用的是临时 Runtime（{}）",
                client.base_url()
            ),
            hint: Some("运行 `else-bot daemon install`，否则后台不会真的替你做事。".into()),
        };
    };
    let mut lines = body.lines();
    let pid = lines
        .next()
        .and_then(|line| line.trim().parse::<i32>().ok());
    let url = lines.next().map(|line| line.trim().to_string());
    let Some(pid) = pid else {
        return Check {
            name: "resident runtime".into(),
            ok: false,
            detail: format!("{} 已损坏；删除它后重新 install", path.display()),
            hint: None,
        };
    };
    if !process_is_alive(pid) {
        return Check {
            name: "resident runtime".into(),
            ok: false,
            detail: format!("常驻 Runtime（pid {pid}）已经退出；这条命令用的是临时 Runtime"),
            hint: Some(
                "看 ~/Library/Logs/else-runtime.log，然后 `else-bot daemon install`。".into(),
            ),
        };
    }
    match url {
        Some(url) if url == client.base_url() => Check {
            name: "resident runtime".into(),
            ok: true,
            detail: format!("常驻 Runtime（pid {pid}）在 {url}"),
            hint: None,
        },
        Some(url) => Check {
            name: "resident runtime".into(),
            ok: false,
            detail: format!(
                "常驻 Runtime 在 {url}（pid {pid}），但这条命令连的是 {}",
                client.base_url()
            ),
            hint: Some("两个 Runtime 会让状态分叉；停掉多余的那个。".into()),
        },
        None => Check {
            name: "resident runtime".into(),
            ok: false,
            detail: format!("{} 没有记录监听地址", path.display()),
            hint: None,
        },
    }
}

#[cfg(unix)]
fn process_is_alive(pid: i32) -> bool {
    // SAFETY: kill 只做信号发送，signal 0 用于探测进程存在性。
    let result = unsafe { libc::kill(pid, 0) };
    result == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

#[cfg(not(unix))]
fn process_is_alive(_pid: i32) -> bool {
    true
}

/// Ledger 可写检查。
fn check_ledger_writable(runtime_dir: &Path) -> Check {
    let probe = runtime_dir.join(".doctor-write-probe");
    match std::fs::write(&probe, b"probe") {
        Ok(()) => {
            let _ = std::fs::remove_file(&probe);
            Check {
                name: "ledger".into(),
                ok: true,
                detail: format!("{} is writable", runtime_dir.display()),
                hint: None,
            }
        }
        Err(error) => Check {
            name: "ledger".into(),
            ok: false,
            detail: format!("{} is not writable: {error}", runtime_dir.display()),
            hint: Some("检查目录权限与磁盘空间。".into()),
        },
    }
}

/// Connector 健康检查。
fn check_connectors(client: &RuntimeClient) -> Check {
    let threads = match client.list_threads() {
        Ok(threads) => threads,
        Err(error) => {
            return Check {
                name: "connectors".into(),
                ok: false,
                detail: format!("cannot read the connector registry: {error}"),
                hint: None,
            }
        }
    };
    let mut total = 0usize;
    let mut unhealthy = Vec::new();
    for thread in threads {
        let connectors = match client.connectors(&thread.spec.id) {
            Ok(connectors) => connectors,
            Err(_) => continue,
        };
        for connector in connectors {
            total += 1;
            if let Ok(health) = client.connector_health(&thread.spec.id, &connector.id) {
                if health.health != else_protocol::connector::ConnectorHealth::Healthy {
                    unhealthy.push(format!("{} ({:?})", connector.adapter, health.health));
                }
            }
        }
    }
    if total == 0 {
        return Check {
            name: "connectors".into(),
            ok: true,
            detail: "no connectors registered".into(),
            hint: None,
        };
    }
    Check {
        name: "connectors".into(),
        ok: unhealthy.is_empty(),
        detail: if unhealthy.is_empty() {
            format!("{total} connector(s), all healthy")
        } else {
            format!("{total} connector(s); unhealthy: {}", unhealthy.join(", "))
        },
        hint: (!unhealthy.is_empty()).then(|| "在 Connectors 界面按 s 可以立即重新同步。".into()),
    }
}
