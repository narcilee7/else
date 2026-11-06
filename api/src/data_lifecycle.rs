//! Explicit user-driven deletion of one Thread.

use serde::Serialize;

use else_protocol::ids::ThreadId;
use else_protocol::thread::ThreadState;

use crate::{ApiError, Result, Runtime};

#[derive(Debug, Serialize)]
pub struct ThreadDeletionReport {
    pub thread_id: ThreadId,
    pub deleted_events: usize,
    pub removed_credentials: usize,
    pub removed_workspace_paths: usize,
    pub cleanup_errors: Vec<String>,
    /// Root workspace / CAS blobs may be shared, so they are not blindly deleted.
    pub retained_shared_artifacts: bool,
}

impl Runtime {
    pub fn delete_thread(&self, thread_id: &ThreadId) -> Result<ThreadDeletionReport> {
        let mut projection = self.projection(thread_id)?;
        if projection.spec.is_none() {
            return Err(ApiError::ThreadNotFound(thread_id.clone()));
        }
        if !matches!(
            projection.state,
            Some(ThreadState::Completed | ThreadState::Failed | ThreadState::Cancelled)
        ) {
            projection = self.abort(thread_id, "user requested permanent deletion")?;
        }
        if !matches!(
            projection.state,
            Some(ThreadState::Completed | ThreadState::Failed | ThreadState::Cancelled)
        ) {
            return Err(ApiError::Conflict(
                "thread did not reach a terminal state before deletion".into(),
            ));
        }

        let mut cleanup_errors = Vec::new();
        let mut removed_credentials = 0;
        for credential in projection
            .connectors
            .values()
            .filter_map(|connector| connector.credential.as_ref())
        {
            match self.credential_store.delete(&credential.reference) {
                Ok(()) => removed_credentials += 1,
                Err(error) => cleanup_errors.push(error.to_string()),
            }
        }

        let mut removed_workspace_paths = 0;
        for scope_id in projection.scopes.keys() {
            for path in [
                self.branches.join(scope_id.to_string()),
                self.scope_workspace(thread_id, scope_id),
            ] {
                match std::fs::remove_dir_all(&path) {
                    Ok(()) => removed_workspace_paths += 1,
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => cleanup_errors.push(format!("{}: {error}", path.display())),
                }
            }
            // 父目录（scopes/<thread_id>）删完子 Scope 后就没用了。
            let _ = std::fs::remove_dir(self.workspace.join("scopes").join(thread_id.to_string()));
        }
        // 只删 Runtime 自己的隔离目录。`--in` 指定的用户目录永远不归 else 删。
        if let Some(spec) = projection
            .spec
            .as_ref()
            .filter(|spec| spec.mode.is_scratch())
        {
            let path = self.thread_workspace(spec)?;
            match std::fs::remove_dir_all(&path) {
                Ok(()) => removed_workspace_paths += 1,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => cleanup_errors.push(format!("{}: {error}", path.display())),
            }
        }

        let deleted_events = self.ledger.delete_thread(thread_id)?;
        Ok(ThreadDeletionReport {
            thread_id: thread_id.clone(),
            deleted_events,
            removed_credentials,
            removed_workspace_paths,
            cleanup_errors,
            retained_shared_artifacts: true,
        })
    }
}
