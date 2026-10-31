//! CLI 与 Runtime 的连接。
//!
//! 终端与 App 必须落在**同一个** Runtime 上，否则用户会看到两个世界。
//! 连接顺序：
//!
//! 1. **复用**已经在运行的 daemon —— macOS App 后台常驻的那一个。
//!    这是「终端和 App 看到同一份 Brief」的路径。
//! 2. 复用不了就在本地起一个**内嵌** daemon，命令结束随之退出。
//!    这样 `else` 零配置可用，同时保证任何时刻只有一个写者。
//!
//! 两种情况走的都是同一条 HTTP 链路 —— CLI 里没有「进程内另一套语义」。

use std::path::{Path, PathBuf};

use else_client::{ClientConfig, RuntimeClient};
use else_daemon::{Daemon, DaemonConfig, DaemonError};

use crate::error::{CliError, Result};

/// 一个已连接的 Runtime 会话。
pub struct Session {
    client: RuntimeClient,
    /// 内嵌 daemon：CLI 自己起的驻留 Runtime。复用外部 daemon 时为 `None`。
    _embedded: Option<Daemon>,
}

impl Session {
    /// 连接 Runtime。
    ///
    /// `dir` 是显式的 Runtime 目录覆盖；缺省用 `ELSE_RUNTIME_DIR`，
    /// 再缺省用平台默认目录（与 macOS App 同址）。
    pub fn open(dir: Option<&Path>) -> Result<Self> {
        let mut config = ClientConfig::from_env();
        if let Some(dir) = dir {
            config.dir = dir.to_path_buf();
        }

        // 1) 复用已在运行的 daemon。
        //
        //    这里用**鉴权**探针而不是公开的 `/v1/health`：后者对任何人都返回 200，
        //    拿它判断「连上了」会把另一个 Runtime（令牌不同）误认成自己的。
        if let Some(client) = attach(&config) {
            return Ok(Self {
                client,
                _embedded: None,
            });
        }
        // 后台 daemon 可能正在启动：锁文件已经在了，但令牌还没落盘。
        // 这种情况下短暂等一下，而不是立刻另起一个 Runtime。
        if config.dir.join("daemon.lock").exists() {
            for _ in 0..10 {
                std::thread::sleep(std::time::Duration::from_millis(200));
                if let Some(client) = attach(&config) {
                    return Ok(Self {
                        client,
                        _embedded: None,
                    });
                }
            }
        }

        // 2) 本地起一个内嵌 daemon。
        let bind = bind_address(&config.url);
        match start_embedded(&config.dir.clone(), bind) {
            Ok(daemon) => {
                let client = RuntimeClient::new(&daemon.url, &daemon.token);
                Ok(Self {
                    client,
                    _embedded: Some(daemon),
                })
            }
            Err(DaemonError::AlreadyRunning(_)) => {
                // 有 daemon 正在跑，但刚才没连上 —— 多半是令牌文件还没落盘，
                // 或者已经过期了。再试一次；还是不行就如实报错。
                if let Some(client) = attach(&config) {
                    return Ok(Self {
                        client,
                        _embedded: None,
                    });
                }
                Err(CliError::Client(else_client::ClientError::Unreachable(
                    format!(
                        "a runtime is already running at {} but the token was rejected",
                        config.url
                    ),
                )))
            }
            Err(DaemonError::Bind(_)) => {
                // 配置的端口被别的进程占了（可能是另一个 Runtime 目录的 daemon）。
                // 换一个临时端口起自己的，绝不误连到别人的 Runtime 上。
                let daemon = start_embedded(&config.dir.clone(), "127.0.0.1:0".to_string())
                    .map_err(|error| {
                        CliError::Client(else_client::ClientError::Unreachable(format!("{error}")))
                    })?;
                let client = RuntimeClient::new(&daemon.url, &daemon.token);
                Ok(Self {
                    client,
                    _embedded: Some(daemon),
                })
            }
            Err(error) => Err(CliError::Client(else_client::ClientError::Unreachable(
                format!("{error}"),
            ))),
        }
    }

    /// Runtime 客户端。
    pub fn client(&self) -> &RuntimeClient {
        &self.client
    }

    /// 交出 Runtime 客户端的所有权。
    pub fn into_client(self) -> RuntimeClient {
        self.client
    }
}

/// 在指定目录起一个内嵌 daemon。
fn start_embedded(dir: &Path, bind: String) -> std::result::Result<Daemon, DaemonError> {
    Daemon::start(DaemonConfig {
        dir: dir.to_path_buf(),
        bind,
        token: None,
    })
}

/// 试着连上已在运行的 Runtime。
///
/// 令牌缺失、地址不通、令牌被拒 —— 都算「没连上」。
fn attach(config: &ClientConfig) -> Option<RuntimeClient> {
    let token = config.resolve_token().ok()?;
    let client = RuntimeClient::new(&config.url, &token);
    client.probe().is_ok().then_some(client)
}

/// 从 Runtime 地址取出监听地址。
///
/// `http://127.0.0.1:4180` → `127.0.0.1:4180`。
fn bind_address(url: &str) -> String {
    let authority = url
        .trim_start_matches("http://")
        .trim_start_matches("https://")
        .trim_end_matches('/');
    authority.to_string()
}

/// 解析 Runtime 目录。
///
/// 优先级：显式 `--dir` > `ELSE_RUNTIME_DIR` > 平台默认目录。
/// macOS 的默认目录与 App 同址 —— 这是终端与 App 共用一个世界的关键。
pub fn resolve_runtime_dir(dir: Option<&Path>) -> PathBuf {
    dir.map(Path::to_path_buf).unwrap_or_else(|| {
        std::env::var_os("ELSE_RUNTIME_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(else_client::default_runtime_dir)
    })
}
