//! Runtime 客户端配置：URL、Runtime 目录与访问令牌。
//!
//! 解析顺序与 macOS 客户端（`ElseClientCore/Configuration/ClientConfiguration.swift`）
//! 完全一致，这样终端与 App 会落到同一个 Runtime 上：
//!
//! 1. `ELSE_RUNTIME_URL` —— 默认 `http://127.0.0.1:4180`。
//! 2. `ELSE_RUNTIME_DIR` —— 默认按平台选择（见 [`default_runtime_dir`]）。
//! 3. `ELSE_RUNTIME_TOKEN` —— 缺省从 `<runtime dir>/daemon-token` 读取。
//!
//! 令牌文件只在 loopback 主机上读取，并且必须是当前用户拥有、权限位不含
//! 组/其他读写的常规文件。远程主机不读本地文件 —— 那里必须显式给令牌。

use std::path::{Path, PathBuf};

use crate::{ClientError, Result};

/// Runtime 默认地址。所有 surface 默认连同一个 loopback daemon。
pub const DEFAULT_RUNTIME_URL: &str = "http://127.0.0.1:4180";

/// Runtime 客户端配置。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientConfig {
    /// Runtime 地址。
    pub url: String,
    /// Runtime 数据目录：事件库、工作区与 `daemon-token` 所在处。
    pub dir: PathBuf,
    /// 显式令牌；缺省时从 `dir/daemon-token` 读取。
    pub token: Option<String>,
}

impl ClientConfig {
    /// 从环境变量解析配置。
    pub fn from_env() -> Self {
        let url = std::env::var("ELSE_RUNTIME_URL")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| DEFAULT_RUNTIME_URL.to_string());
        let dir = std::env::var("ELSE_RUNTIME_DIR")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(default_runtime_dir);
        let token = std::env::var("ELSE_RUNTIME_TOKEN")
            .ok()
            .filter(|value| !value.trim().is_empty());
        Self { url, dir, token }
    }

    /// 构造一个指向指定 Runtime 目录的配置。
    pub fn for_dir(dir: impl Into<PathBuf>) -> Self {
        Self {
            url: DEFAULT_RUNTIME_URL.to_string(),
            dir: dir.into(),
            token: None,
        }
    }

    /// 令牌文件路径。
    pub fn token_path(&self) -> PathBuf {
        self.dir.join("daemon-token")
    }

    /// 解析出可用的访问令牌。
    ///
    /// 显式令牌优先；否则在 loopback 主机上读令牌文件。
    pub fn resolve_token(&self) -> Result<String> {
        if let Some(token) = self.token.as_ref() {
            return Ok(token.clone());
        }
        if !is_loopback(&self.url) {
            return Err(ClientError::missing_token());
        }
        load_token(&self.token_path())
    }
}

/// Runtime 数据目录的跨平台默认值。
///
/// macOS 与 App 的 `Application Support/else/runtime` 同址 —— 这是
/// 「终端和 App 看到同一个世界」的前提。Linux 走 XDG。
pub fn default_runtime_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("XDG_DATA_HOME").filter(|value| !value.is_empty()) {
        return PathBuf::from(dir).join("else").join("runtime");
    }
    #[cfg(target_os = "macos")]
    {
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home)
                .join("Library")
                .join("Application Support")
                .join("else")
                .join("runtime");
        }
    }
    #[cfg(target_os = "linux")]
    {
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home)
                .join(".local")
                .join("share")
                .join("else")
                .join("runtime");
        }
    }
    PathBuf::from(".else")
}

/// 读取 Runtime 令牌文件。
///
/// 只接受常规文件、当前用户拥有、权限位不含组/其他读写的文件。
/// 宁可读不到令牌，也不要读一个可能被同机其他用户替换的文件。
fn load_token(path: &Path) -> Result<String> {
    let metadata = std::fs::metadata(path)
        .map_err(|error| ClientError::MissingToken(format!("{}: {error}", path.display())))?;
    if !metadata.is_file() {
        return Err(ClientError::MissingToken(format!(
            "{} is not a regular file",
            path.display()
        )));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        use std::os::unix::fs::PermissionsExt;
        if metadata.uid() != current_uid() {
            return Err(ClientError::MissingToken(format!(
                "{} is owned by another user",
                path.display()
            )));
        }
        if metadata.permissions().mode() & 0o077 != 0 {
            return Err(ClientError::MissingToken(format!(
                "{} is readable by group or others",
                path.display()
            )));
        }
    }
    let contents = std::fs::read_to_string(path)
        .map_err(|error| ClientError::MissingToken(format!("{}: {error}", path.display())))?;
    let token = contents.trim();
    if token.is_empty() {
        return Err(ClientError::MissingToken(format!(
            "{} is empty",
            path.display()
        )));
    }
    Ok(token.to_string())
}

#[cfg(unix)]
fn current_uid() -> u32 {
    // SAFETY: `getuid` 没有参数、没有内存影响，任何时候都可以安全调用。
    unsafe { libc::getuid() }
}

fn is_loopback(url: &str) -> bool {
    let host = url
        .trim_start_matches("http://")
        .trim_start_matches("https://")
        .split(['/', ':'])
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    matches!(host.as_str(), "127.0.0.1" | "localhost" | "::1" | "[::1]")
}
