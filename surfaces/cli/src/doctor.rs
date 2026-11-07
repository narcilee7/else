//! `else-bot doctor`：一次说清「现在这台机器上的 else 是什么状态」。
//!
//! 出问题时，用户不该去翻日志猜。把该看的都看一遍，然后给出一份
//! **可以直接贴给别人的**报告。

use std::path::{Path, PathBuf};

use else_client::{RuntimeClient, RuntimeVersion};
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
///
/// 顺序是有讲究的：先弄清"在这个端口应答的到底是谁"，再谈它的健康。
/// 端口被别人的进程占着时，后面每一项都会报出误导性的结论 ——
/// 这也是最常发生的一种。
pub fn run(client: &RuntimeClient, runtime_dir: &Path) -> DoctorReport {
    diagnose(client, runtime_dir).report
}

/// 一次诊断的完整结果：报告 + "端口上应答的是谁"。
struct Diagnosis {
    report: DoctorReport,
    identity: Option<Identity>,
}

fn diagnose(client: &RuntimeClient, runtime_dir: &Path) -> Diagnosis {
    let mut checks = Vec::new();

    // 1) Runtime 目录。
    checks.push(check_runtime_dir(runtime_dir));

    // 2) 后台服务。
    checks.push(check_service(runtime_dir));

    // 3) 在这个端口应答的是谁。
    let identity = check_identity(client, runtime_dir);
    let ours = identity.as_ref().is_some_and(|identity| identity.ours);
    checks.push(identity_check(identity.as_ref(), client));

    // 4) 当前会话接的到底是常驻 Runtime 还是临时起的一个。
    checks.push(check_resident(client, runtime_dir, identity.as_ref()));

    // 5) 令牌有效性 —— 与「可达」是两件事。
    let token_ok = match client.probe() {
        Ok(()) => {
            checks.push(Check {
                name: "token".into(),
                ok: true,
                detail: "the token belongs to this runtime".into(),
                hint: None,
            });
            true
        }
        Err(error) => {
            checks.push(Check {
                name: "token".into(),
                ok: false,
                detail: format!("the token was rejected: {error}"),
                hint: Some("删除 runtime 目录里的 daemon-token 后重启 Runtime。".into()),
            });
            false
        }
    };

    // 6) 它到底实现了哪些路由。旧构建会自称同一个契约版本，
    //    然后在真正的调用上返回 404。
    let routes_ok = if ours && token_ok {
        let (check, all_present) = check_routes(client);
        checks.push(check);
        all_present
    } else {
        checks.push(skipped(
            "routes",
            "skipped: the runtime answering this port is not the one this command manages",
        ));
        false
    };

    // 7) 这台机器能把 agent 关进什么样的盒子里。
    checks.push(check_sandbox());

    // 8) Ledger 可写。
    checks.push(check_ledger_writable(runtime_dir));

    // 9) Connector 健康。
    checks.push(if routes_ok {
        check_connectors(client)
    } else {
        skipped(
            "connectors",
            "skipped: the runtime did not answer every route this build needs",
        )
    });

    Diagnosis {
        report: DoctorReport {
            runtime_dir: runtime_dir.to_path_buf(),
            runtime_url: client.base_url().to_string(),
            checks,
        },
        identity,
    }
}

/// 一次自愈尝试的结果。
pub struct FixOutcome {
    /// 实际做了什么。
    pub actions: Vec<String>,
    /// 修不了的部分（有值就说明还没好）。
    pub blocked: Option<String>,
    /// 修完之后重新诊断的报告。
    pub report: DoctorReport,
}

/// 能自己修的就自己修，修不了的说清楚卡在哪。
///
/// 只动自己的东西：自己的服务定义、自己的常驻进程。别人的进程一律只报
/// pid，不替用户杀。
pub fn fix(client: &RuntimeClient, runtime_dir: &Path, dry_run: bool) -> FixOutcome {
    let diagnosis = diagnose(client, runtime_dir);
    let holder = diagnosis
        .identity
        .as_ref()
        .filter(|identity| !identity.ours)
        .map(Identity::describe);
    let status = service::status(runtime_dir);
    let current = std::env::current_exe().ok();

    let mut actions = Vec::new();
    let mut blocked = None;
    match service::plan_repair(&status, current.as_deref(), runtime_dir, holder.as_deref()) {
        service::Repair::Nothing => {}
        service::Repair::Reinstall(reasons) => {
            actions.extend(reasons.into_iter().map(|reason| format!("found: {reason}")));
            if dry_run {
                actions.push("would reinstall the background service".to_string());
            } else {
                match service::install(runtime_dir) {
                    Ok(report) => actions.extend(report.actions),
                    Err(error) => blocked = Some(error.to_string()),
                }
            }
        }
        service::Repair::CannotFix(reason) => blocked = Some(reason),
    }

    FixOutcome {
        actions,
        blocked,
        report: run(client, runtime_dir),
    }
}

fn skipped(name: &str, detail: &str) -> Check {
    Check {
        name: name.into(),
        ok: false,
        detail: detail.into(),
        hint: None,
    }
}

/// 在这个端口应答的 Runtime 是谁。
struct Identity {
    version: RuntimeVersion,
    /// 它替的就是这个命令所管的那个目录，并且 pid 与登记一致。
    ours: bool,
    /// 与登记不一致的具体原因（`ours` 为真时是 None）。
    mismatch: Option<String>,
}

fn check_identity(client: &RuntimeClient, runtime_dir: &Path) -> Option<Identity> {
    let version = client.version().ok()?;
    let recorded = recorded_pid(runtime_dir);

    // 旧版 Runtime 不报 pid 与目录：能确认的只有契约版本。
    let served_dir = version.runtime_dir.clone();
    let Some(served_dir) = served_dir else {
        return Some(Identity {
            version,
            ours: false,
            mismatch: Some("它不报自己替哪个目录服务（这个 Runtime 比当前命令旧）".to_string()),
        });
    };
    if Path::new(&served_dir) != runtime_dir {
        return Some(Identity {
            version,
            ours: false,
            mismatch: Some(format!(
                "它替 {served_dir} 服务，而这条命令管的是 {}",
                runtime_dir.display()
            )),
        });
    }
    if let (Some(answered), Some(recorded)) = (version.pid, recorded) {
        if answered != recorded {
            return Some(Identity {
                version,
                ours: false,
                mismatch: Some(format!(
                    "端口由进程 {answered} 应答，但 daemon.pid 里登记的是 {recorded}"
                )),
            });
        }
    }
    Some(Identity {
        version,
        ours: true,
        mismatch: None,
    })
}

impl Identity {
    /// 让人能拿这个描述去指认进程。
    fn describe(&self) -> String {
        let pid = self
            .version
            .pid
            .map(|pid| format!("pid {pid}"))
            .unwrap_or_else(|| "一个不报 pid 的进程".to_string());
        match self.mismatch.as_deref() {
            Some(reason) => format!("{pid}（{reason}）"),
            None => pid,
        }
    }
}

fn identity_check(identity: Option<&Identity>, client: &RuntimeClient) -> Check {
    let Some(identity) = identity else {
        return Check {
            name: "runtime".into(),
            ok: false,
            detail: format!("cannot reach the runtime at {}", client.base_url()),
            hint: Some(
                "运行 `else-bot daemon install` 让 Runtime 常驻，或用 `else-bot serve` 前台启动。"
                    .into(),
            ),
        };
    };
    let version = &identity.version;
    let contract = format!(
        "contract {}, protocol {}",
        version.contract_version, version.protocol_version
    );
    match &identity.mismatch {
        None => Check {
            name: "runtime".into(),
            ok: true,
            detail: match version.pid {
                Some(pid) => format!(
                    "reachable at {} · pid {pid} · {contract}",
                    client.base_url()
                ),
                None => format!("reachable at {} · {contract}", client.base_url()),
            },
            hint: None,
        },
        Some(reason) => Check {
            name: "runtime".into(),
            ok: false,
            detail: format!(
                "{} 上应答的不是这条命令的 Runtime：{reason}",
                client.base_url()
            ),
            hint: Some(match version.pid {
                Some(pid) => format!(
                    "先停掉那个进程（pid {pid}），再 `else-bot daemon install` —— 端口被占时它装不上。"
                ),
                None => "运行 `else-bot doctor --fix`：它会把后台服务指回这个二进制并重启；\
                         若端口是被别人的进程占着，它只会告诉你那个进程，不会替你杀。"
                    .into(),
            }),
        },
    }
}

/// `daemon.pid` 里登记的 pid（常驻 Runtime 才会写）。
fn recorded_pid(runtime_dir: &Path) -> Option<u32> {
    let body = std::fs::read_to_string(runtime_dir.join("daemon.pid")).ok()?;
    body.lines().next()?.trim().parse().ok()
}

/// 它到底实现了哪些路由。
///
/// 契约版本只能证明"协议对得上"，证明不了"路由都在"。这里逐个真调用一次
/// 不需要参数的接口，再把有线程时才能探的那条也探一次 —— 缺哪条就报哪条。
fn check_routes(client: &RuntimeClient) -> (Check, bool) {
    let mut probes: Vec<(&str, bool)> = vec![
        ("threads", client.list_threads().is_ok()),
        ("world/brief", client.brief().is_ok()),
        ("attention", client.attention_windows().is_ok()),
        ("harnesses", client.harnesses().is_ok()),
        ("projects", client.project_worlds(None, false).is_ok()),
        ("identities", client.identity_worlds(None, false).is_ok()),
        ("diagnostics", client.diagnostics().is_ok()),
        ("settings", client.settings().is_ok()),
    ];

    let threads = client.list_threads().unwrap_or_default();
    if let Some(thread) = threads.first() {
        probes.push((
            "thread connectors",
            client.connectors(&thread.spec.id).is_ok(),
        ));
    }

    let missing: Vec<&str> = probes
        .iter()
        .filter(|(_, ok)| !ok)
        .map(|(name, _)| *name)
        .collect();
    let total = probes.len();
    if missing.is_empty() {
        return (
            Check {
                name: "routes".into(),
                ok: true,
                detail: format!("all {total} probed routes answer"),
                hint: None,
            },
            true,
        );
    }
    (
        Check {
            name: "routes".into(),
            ok: false,
            detail: format!(
                "这个 Runtime 缺少 {} 条路由：{}",
                missing.len(),
                missing.join(", ")
            ),
            hint: Some(
                "它比当前 else-bot 旧（或根本不是这个构建）。停掉它，再 `else-bot daemon install`。"
                    .into(),
            ),
        },
        false,
    )
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
            // 服务注册在另一个安装路径上时，一切都像是正常的 —— 端口上跑的
            // 其实是另一个程序。这句话就是那种情况唯一的线索。
            detail: with_program_note(status.detail, status.unit_program.as_deref()),
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

/// 服务启动的程序与这条命令是不是同一个。
fn with_program_note(detail: String, program: Option<&Path>) -> String {
    let Some(program) = program else {
        return detail;
    };
    let Ok(current) = std::env::current_exe() else {
        return detail;
    };
    if current == program {
        return detail;
    }
    format!(
        "{detail}；服务启动的是 {}，这条命令是 {}",
        program.display(),
        current.display()
    )
}

/// 当前会话接的是常驻 Runtime 还是命令自己起的临时 Runtime。
///
/// 常驻 daemon 启动时会写 `daemon.pid`（pid + 监听地址）。没有它，或者地址
/// 不匹配，就说明这条命令连的是一个用完即弃的替身 —— 连接器同步、定时器与
/// 通知都不会发生，而用户看到的仍是"一切正常"。
fn check_resident(
    client: &RuntimeClient,
    runtime_dir: &Path,
    identity: Option<&Identity>,
) -> Check {
    let path = runtime_dir.join("daemon.pid");
    let Ok(body) = std::fs::read_to_string(&path) else {
        // 没有登记。如果在这个端口应答的就是这个目录的进程，那说明它
        // 是个常驻 Runtime 但登记丢了；否则就是命令自己起的临时替身。
        let detail = match identity.and_then(|identity| identity.version.pid) {
            Some(pid) if identity.is_some_and(|identity| identity.ours) => {
                format!("进程 {pid} 正在服务这个目录，但没有 daemon.pid 登记；它可能是前台启动的")
            }
            _ => format!(
                "没有常驻 Runtime；这条命令用的是临时 Runtime（{}）",
                client.base_url()
            ),
        };
        return Check {
            name: "resident runtime".into(),
            ok: false,
            detail,
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

/// 沙箱能力。
///
/// 平台上跑得起来，不等于能力一样：Linux 的 bubblewrap 兑现不了"出网走本地
/// 代理 + 白名单"这条策略，带网络的 opaque harness 在那里会被拒绝执行。
/// 这种差异必须由 `doctor` 说出来，而不是等用户跑到一半才撞上。
fn check_sandbox() -> Check {
    let backend = match else_environment::Backend::native() {
        Ok(backend) => backend,
        Err(error) => {
            return Check {
                name: "sandbox".into(),
                ok: false,
                detail: format!("no sandbox backend on this platform: {error}"),
                hint: Some(
                    "没有沙箱就不会执行任何 agent —— else 不会在没有边界的情况下跑你的工具。"
                        .into(),
                ),
            }
        }
    };
    if backend.supports_local_proxy() {
        return Check {
            name: "sandbox".into(),
            ok: true,
            detail: format!(
                "{} · network goes through a local allowlist proxy",
                backend.name()
            ),
            hint: None,
        };
    }
    // 这不是失败，是这台机器的能力边界 —— 所以要写进 detail，
    // 而不是放进只在失败时才打印的 hint 里。
    Check {
        name: "sandbox".into(),
        ok: true,
        detail: format!(
            "{} · networked harnesses are refused (no local allowlist proxy on this backend)",
            backend.name()
        ),
        hint: None,
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
