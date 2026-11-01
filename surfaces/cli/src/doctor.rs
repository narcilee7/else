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

    // 3) Runtime 可达性与契约版本。
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
fn check_service(runtime_dir: &Path) -> Check {
    let status = service::status(runtime_dir);
    Check {
        name: "background service".into(),
        ok: status.loaded,
        detail: status.detail,
        hint: (!status.loaded).then(|| "运行 `else-bot daemon install` 注册后台服务。".into()),
    }
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
    let intents = match client.list_intents() {
        Ok(intents) => intents,
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
    for intent in intents {
        let connectors = match client.connectors(&intent.spec.id) {
            Ok(connectors) => connectors,
            Err(_) => continue,
        };
        for connector in connectors {
            total += 1;
            if let Ok(health) = client.connector_health(&intent.spec.id, &connector.id) {
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
