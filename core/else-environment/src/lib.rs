//! Environment / Sandbox：易失执行环境与持久 Workspace 分离。
//!
//! M1 只提供本地强隔离后端：macOS Seatbelt、Linux bubblewrap。
//! 文件系统默认拒绝，仅开放系统只读运行路径与 Workspace 读写；
//! 网络默认拒绝，环境变量清空，后端不可用时失败关闭，不回退宿主机执行。

mod bubblewrap;
mod seatbelt;

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetworkPolicy {
    Deny,
    /// 只允许连接 Runtime 的 loopback CONNECT 代理端口。
    Proxy {
        port: u16,
        #[allow(dead_code)]
        tool_gateway_port: Option<u16>,
    },
    /// 放通全部出站（MVP-2：Harness 原生 Provider 流量）。
    /// 这是显式策略选择，不是默认；按域名收敛后置，使用方必须声明 Opaque。
    AllowOutbound,
}

/// 同一 Profile 后续可由本地或云后端解析；M1 不开放自定义例外。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SandboxProfile {
    pub workspace: PathBuf,
    pub network: NetworkPolicy,
    /// 额外只读路径（Harness 可执行文件及其运行时依赖，如 Homebrew）。
    /// 只读不写；调用方必须显式声明，不存在隐式放行。
    pub read_paths: Vec<PathBuf>,
    /// 额外可写路径。只用于 Runtime 控制的私有 Harness 目录，
    /// 不能用来扩大用户 Workspace 的写入范围。
    pub write_paths: Vec<PathBuf>,
    /// Opaque Harness 使用 Tool Gateway 时，父 Harness Workspace 必须只读。
    pub workspace_writable: bool,
}

impl SandboxProfile {
    pub fn new(workspace: impl Into<PathBuf>) -> Self {
        Self {
            workspace: workspace.into(),
            network: NetworkPolicy::Deny,
            read_paths: vec![],
            write_paths: vec![],
            workspace_writable: true,
        }
    }

    /// 显式放通出站网络（Harness 原生模型流量）；默认拒绝不变。
    pub fn with_network(mut self, network: NetworkPolicy) -> Self {
        self.network = network;
        self
    }

    /// 追加只读路径（Harness 二进制与运行时依赖）。
    pub fn with_read_paths(mut self, paths: Vec<PathBuf>) -> Self {
        self.read_paths = paths;
        self
    }

    /// 追加 Runtime 管理的私有可写目录。
    pub fn with_write_paths(mut self, paths: Vec<PathBuf>) -> Self {
        self.write_paths = paths;
        self
    }

    pub fn with_workspace_read_only(mut self) -> Self {
        self.workspace_writable = false;
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    Seatbelt,
    Bubblewrap,
}

impl Backend {
    pub fn native() -> Result<Self> {
        #[cfg(target_os = "macos")]
        return Ok(Self::Seatbelt);
        #[cfg(target_os = "linux")]
        return Ok(Self::Bubblewrap);
        #[cfg(not(any(target_os = "macos", target_os = "linux")))]
        return Err(SandboxError::UnsupportedPlatform(
            std::env::consts::OS.into(),
        ));
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Seatbelt => "seatbelt",
            Self::Bubblewrap => "bubblewrap",
        }
    }

    fn executable(self) -> Option<PathBuf> {
        let candidates: &[&str] = match self {
            Self::Seatbelt => &["/usr/bin/sandbox-exec"],
            Self::Bubblewrap => &["/usr/bin/bwrap", "/bin/bwrap"],
        };
        candidates
            .iter()
            .map(PathBuf::from)
            .find(|path| path.is_file())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SandboxError {
    #[error("sandbox backend unavailable: {backend}; refusing unsandboxed execution")]
    BackendUnavailable { backend: &'static str },
    #[error("sandbox unsupported on platform: {0}")]
    UnsupportedPlatform(String),
    #[error("invalid sandbox profile: {0}")]
    InvalidProfile(String),
    #[error("network proxy is not enforceable on backend: {backend}")]
    ProxyUnsupported { backend: &'static str },
    #[error("sandbox io: {0}")]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, SandboxError>;

#[derive(Debug)]
pub struct Sandbox {
    backend: Backend,
    executable: PathBuf,
    profile: SandboxProfile,
}

impl Sandbox {
    /// 选择当前平台的原生后端。不存在隐式无沙箱后端。
    pub fn new(profile: SandboxProfile) -> Result<Self> {
        Self::with_backend(profile, Backend::native()?)
    }

    /// 显式后端便于测试与未来的环境解析；同样校验真实可执行文件。
    pub fn with_backend(mut profile: SandboxProfile, backend: Backend) -> Result<Self> {
        let executable = backend
            .executable()
            .ok_or(SandboxError::BackendUnavailable {
                backend: backend.name(),
            })?;
        profile.workspace = profile.workspace.canonicalize().map_err(|error| {
            SandboxError::InvalidProfile(format!(
                "workspace {}: {error}",
                profile.workspace.display()
            ))
        })?;
        if !profile.workspace.is_dir() {
            return Err(SandboxError::InvalidProfile(format!(
                "workspace is not a directory: {}",
                profile.workspace.display()
            )));
        }
        for path in &mut profile.write_paths {
            *path = path.canonicalize().map_err(|error| {
                SandboxError::InvalidProfile(format!(
                    "private write path {}: {error}",
                    path.display()
                ))
            })?;
            if !path.is_dir() {
                return Err(SandboxError::InvalidProfile(format!(
                    "private write path is not a directory: {}",
                    path.display()
                )));
            }
        }
        if backend == Backend::Seatbelt && profile.workspace.to_str().is_none() {
            return Err(SandboxError::InvalidProfile(
                "Seatbelt workspace path must be valid UTF-8".into(),
            ));
        }
        if backend == Backend::Bubblewrap && matches!(profile.network, NetworkPolicy::Proxy { .. })
        {
            return Err(SandboxError::ProxyUnsupported {
                backend: backend.name(),
            });
        }
        Ok(Self {
            backend,
            executable,
            profile,
        })
    }

    pub fn backend(&self) -> Backend {
        self.backend
    }

    pub fn workspace(&self) -> &Path {
        &self.profile.workspace
    }

    /// 在沙箱内执行 shell；进程启动失败与后端缺失都是错误，绝不直接调用宿主机 sh。
    pub fn run_shell(&self, shell_command: &str) -> Result<Output> {
        Ok(self.base_command(shell_command, &[])?.output()?)
    }

    /// 一次性协议调用：写入 stdin 后收集 stdout/stderr。
    pub fn run_shell_with_input(&self, shell_command: &str, input: &str) -> Result<Output> {
        use std::io::Write;
        let mut child = self
            .base_command(shell_command, &[])?
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()?;
        if let Some(mut stdin) = child.stdin.take() {
            stdin.write_all(input.as_bytes())?;
        }
        Ok(child.wait_with_output()?)
    }

    /// 在沙箱内启动长生命周期进程（Harness）：stdout/stderr 接管给调用方流式读取。
    /// `env` 是按任务显式注入的凭据白名单；沙箱默认清空环境，不继承宿主变量。
    /// 取消语义为 kill：进程死亡即沙箱销毁，没有需要单独回收的环境状态。
    pub fn spawn_shell(
        &self,
        shell_command: &str,
        env: &[(String, String)],
    ) -> Result<std::process::Child> {
        let mut command = self.base_command(shell_command, env)?;
        command
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        Ok(command.spawn()?)
    }

    fn base_command(&self, shell_command: &str, env: &[(String, String)]) -> Result<Command> {
        let mut command = match self.backend {
            Backend::Seatbelt => seatbelt::command(
                &self.executable,
                &self.profile.workspace,
                shell_command,
                self.profile.network,
                &self.profile.read_paths,
                &self.profile.write_paths,
                self.profile.workspace_writable,
            )?,
            Backend::Bubblewrap => bubblewrap::command(
                &self.executable,
                &self.profile.workspace,
                shell_command,
                self.profile.network,
                &self.profile.read_paths,
                &self.profile.write_paths,
                self.profile.workspace_writable,
            ),
        };
        command
            .current_dir(&self.profile.workspace)
            .env_clear()
            .env("HOME", &self.profile.workspace)
            .env("TMPDIR", &self.profile.workspace)
            .env("PATH", "/usr/bin:/bin:/usr/sbin:/sbin")
            .env("LC_ALL", "C");
        for (key, value) in env {
            command.env(key, value);
        }
        Ok(command)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unavailable_backend_fails_closed() {
        let backend = if cfg!(target_os = "macos") {
            Backend::Bubblewrap
        } else {
            Backend::Seatbelt
        };
        if backend.executable().is_some() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let error = Sandbox::with_backend(SandboxProfile::new(dir.path()), backend).unwrap_err();
        assert!(matches!(error, SandboxError::BackendUnavailable { .. }));
        assert!(error.to_string().contains("refusing unsandboxed execution"));
    }
}
