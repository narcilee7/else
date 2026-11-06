//! Thread 的执行位置：Scratch 隔离目录，或用户指定的真实目录。
//!
//! 只做两件事：把 `ThreadMode` 解析成一个可用的路径，并在创建时拒绝
//! 用不了的路径。不在这里做权限判断——那是 Sandbox 与 Authority 的事。

use std::path::{Path, PathBuf};

use else_protocol::thread::{ThreadMode, ThreadSpec};

use crate::{ApiError, Result, Runtime};

impl Runtime {
    /// 某个 Thread 实际去哪里干活。
    ///
    /// - `Scratch`：`<runtime>/workspace/threads/<thread_id>`，每个 Thread 一个，
    ///   删 Thread 就删目录，不会误伤别的工作。
    /// - `Directory`：用户给的路径本身。Runtime 不复制、不拥有它。
    pub fn thread_workspace(&self, spec: &ThreadSpec) -> Result<PathBuf> {
        match &spec.mode {
            ThreadMode::Scratch => Ok(self.workspace.join("threads").join(spec.id.to_string())),
            ThreadMode::Directory { path } => resolve_directory(path),
        }
    }

    /// 从投影取执行位置；没有 spec 时退回 Runtime 的工作区。
    pub(crate) fn workspace_of(
        &self,
        projection: &else_ledger::ThreadProjection,
    ) -> Result<PathBuf> {
        match projection.spec.as_ref() {
            Some(spec) => self.thread_workspace(spec),
            None => Ok(self.workspace.clone()),
        }
    }

    /// 子 Scope 的执行目录：始终在 Runtime 下，与用户目录无关。
    /// 子 Scope 是 else 的内部隔离，不应该在用户的仓库里留下目录。
    pub(crate) fn scope_workspace(
        &self,
        thread_id: &else_protocol::ids::ThreadId,
        scope_id: &else_protocol::ids::ScopeId,
    ) -> PathBuf {
        self.workspace
            .join("scopes")
            .join(thread_id.to_string())
            .join(scope_id.to_string())
    }

    /// 创建时校验：路径必须是绝对、存在、且是目录。
    /// 不隐式创建目录 —— 写错一个字母，它应该报错而不是在你机器上造一个新目录。
    pub(crate) fn validate_thread_mode(mode: &ThreadMode) -> Result<()> {
        match mode {
            ThreadMode::Scratch => Ok(()),
            ThreadMode::Directory { path } => resolve_directory(path).map(|_| ()),
        }
    }
}

fn resolve_directory(path: &str) -> Result<PathBuf> {
    let candidate = Path::new(path);
    if !candidate.is_absolute() {
        return Err(ApiError::InvalidThreadMode(format!(
            "{path} is not an absolute path"
        )));
    }
    if !candidate.is_dir() {
        return Err(ApiError::InvalidThreadMode(format!(
            "{path} is not an existing directory"
        )));
    }
    // 规范化：后续所有路径比较（工件归属、相对路径）都要建立在真实路径上。
    Ok(candidate.canonicalize()?)
}
