//! Explicit user-driven deletion of one Intent.

use serde::Serialize;

use else_protocol::ids::IntentId;
use else_protocol::intent::IntentState;

use crate::{ApiError, Result, Runtime};

#[derive(Debug, Serialize)]
pub struct IntentDeletionReport {
    pub intent_id: IntentId,
    pub deleted_events: usize,
    pub removed_credentials: usize,
    pub removed_workspace_paths: usize,
    pub cleanup_errors: Vec<String>,
    /// Root workspace / CAS blobs may be shared, so they are not blindly deleted.
    pub retained_shared_artifacts: bool,
}

impl Runtime {
    pub fn delete_intent(&self, intent_id: &IntentId) -> Result<IntentDeletionReport> {
        let mut projection = self.projection(intent_id)?;
        if projection.spec.is_none() {
            return Err(ApiError::IntentNotFound(intent_id.clone()));
        }
        if !matches!(
            projection.state,
            Some(IntentState::Completed | IntentState::Failed | IntentState::Cancelled)
        ) {
            projection = self.abort(intent_id, "user requested permanent deletion")?;
        }
        if !matches!(
            projection.state,
            Some(IntentState::Completed | IntentState::Failed | IntentState::Cancelled)
        ) {
            return Err(ApiError::Conflict(
                "intent did not reach a terminal state before deletion".into(),
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
                self.workspace.join("scopes").join(scope_id.to_string()),
            ] {
                match std::fs::remove_dir_all(&path) {
                    Ok(()) => removed_workspace_paths += 1,
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => cleanup_errors.push(format!("{}: {error}", path.display())),
                }
            }
        }

        let deleted_events = self.ledger.delete_intent(intent_id)?;
        Ok(IntentDeletionReport {
            intent_id: intent_id.clone(),
            deleted_events,
            removed_credentials,
            removed_workspace_paths,
            cleanup_errors,
            retained_shared_artifacts: true,
        })
    }
}
