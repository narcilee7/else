//! Environment / Sandbox：易失执行环境与持久 Workspace 分离。
//!
//! M1 只提供本地强隔离后端：macOS Seatbelt、Linux bubblewrap。
//! 文件系统默认拒绝，仅开放系统只读运行路径与 Workspace 读写；
//! 网络默认拒绝，环境变量清空，后端不可用时失败关闭，不回退宿主机执行。

mod bubblewrap;
mod seatbelt;

use std::path::{Path, PathBuf};
use std::process::Output;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetworkPolicy {
    Deny,
}

/// 同一 Profile 后续可由本地或云后端解析；M1 不开放自定义例外。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SandboxProfile {
    pub workspace: PathBuf,
    pub network: NetworkPolicy,
}

impl SandboxProfile {
    pub fn new(workspace: impl Into<PathBuf>) -> Self {
        Self {
            workspace: workspace.into(),
            network: NetworkPolicy::Deny,
        }
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
        if backend == Backend::Seatbelt && profile.workspace.to_str().is_none() {
            return Err(SandboxError::InvalidProfile(
                "Seatbelt workspace path must be valid UTF-8".into(),
            ));
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
        let mut command = match self.backend {
            Backend::Seatbelt => {
                seatbelt::command(&self.executable, &self.profile.workspace, shell_command)?
            }
            Backend::Bubblewrap => {
                bubblewrap::command(&self.executable, &self.profile.workspace, shell_command)
            }
        };
        command
            .current_dir(&self.profile.workspace)
            .env_clear()
            .env("HOME", &self.profile.workspace)
            .env("TMPDIR", &self.profile.workspace)
            .env("PATH", "/usr/bin:/bin:/usr/sbin:/sbin")
            .env("LC_ALL", "C");
        Ok(command.output()?)
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
