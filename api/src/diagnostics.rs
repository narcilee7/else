//! Redacted runtime diagnostics suitable for local support bundles.

use std::collections::BTreeMap;
use std::path::Path;

use chrono::{DateTime, Utc};
use else_protocol::connector::ConnectorHealth;
use else_protocol::event::ScopeState;
use else_protocol::ids::{ConnectorId, IntentId, ScopeId};
use else_protocol::intent::IntentState;
use else_protocol::PROTOCOL_VERSION;
use serde::Serialize;

use crate::{Result, Runtime};

#[derive(Debug, Serialize)]
pub struct RuntimeDiagnosticReport {
    pub generated_at: DateTime<Utc>,
    pub runtime_version: String,
    pub protocol_version: String,
    pub healthy: bool,
    pub database_integrity: String,
    pub event_count: u64,
    pub event_cursor: u64,
    pub intent_count: u64,
    pub intent_states: BTreeMap<String, u64>,
    pub connectors: Vec<ConnectorDiagnostic>,
    pub scopes: Vec<ScopeDiagnostic>,
    pub storage: RuntimeStorageDiagnostic,
    pub credential_backend: String,
    pub retention_days: Option<u64>,
}

#[derive(Debug, Serialize)]
pub struct ConnectorDiagnostic {
    pub id: ConnectorId,
    pub intent_id: IntentId,
    pub name: String,
    pub adapter: String,
    pub health: ConnectorHealth,
    pub credential_configured: bool,
    pub last_success_at: Option<DateTime<Utc>>,
    pub consecutive_failures: u32,
    pub last_error: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ScopeDiagnostic {
    pub intent_id: IntentId,
    pub scope_id: ScopeId,
    pub parent_scope_id: Option<ScopeId>,
    pub state: ScopeState,
    pub generation: u64,
    pub failure_count: u32,
    pub last_failure_code: Option<String>,
    pub last_failure_message: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct RuntimeStorageDiagnostic {
    pub database_bytes: u64,
    pub workspace_bytes: u64,
    pub artifact_store_bytes: u64,
    pub branch_bytes: u64,
}

impl Runtime {
    pub fn diagnostics(&self) -> Result<RuntimeDiagnosticReport> {
        let database_integrity = self.ledger.integrity_check()?;
        let event_count = self.ledger.event_count()?;
        let event_cursor = self.ledger.cursor()?;
        let specs = self.list_intents()?;
        let mut intent_states = BTreeMap::new();
        let mut connectors = Vec::new();
        let mut scopes = Vec::new();
        for spec in &specs {
            let projection = self.projection(&spec.id)?;
            let state = projection.state.unwrap_or(IntentState::Active);
            *intent_states.entry(state_name(state).into()).or_insert(0) += 1;
            for connector in projection.connectors.values() {
                let health = projection
                    .connector_sync_status
                    .get(&connector.id)
                    .map(|status| status.health())
                    .unwrap_or(connector.manifest.health);
                let status = projection.connector_sync_status.get(&connector.id);
                connectors.push(ConnectorDiagnostic {
                    id: connector.id.clone(),
                    intent_id: spec.id.clone(),
                    name: connector.name.clone(),
                    adapter: connector.adapter.clone(),
                    health,
                    credential_configured: connector.credential.is_some(),
                    last_success_at: status.and_then(|status| status.last_success_at),
                    consecutive_failures: status
                        .map(|status| status.consecutive_failures)
                        .unwrap_or(0),
                    last_error: status.and_then(|status| status.last_error.clone()),
                });
            }
            for (scope_id, scope) in &projection.scopes {
                scopes.push(ScopeDiagnostic {
                    intent_id: spec.id.clone(),
                    scope_id: scope_id.clone(),
                    parent_scope_id: scope.parent.clone(),
                    state: scope.state,
                    generation: scope.generation,
                    failure_count: scope.failure_count,
                    last_failure_code: scope
                        .last_failure
                        .as_ref()
                        .map(|failure| failure.code.clone()),
                    last_failure_message: scope
                        .last_failure
                        .as_ref()
                        .map(|failure| failure.message.clone()),
                });
            }
        }
        connectors.sort_by(|left, right| {
            left.intent_id
                .cmp(&right.intent_id)
                .then_with(|| left.name.cmp(&right.name))
        });
        scopes.sort_by(|left, right| {
            left.intent_id
                .cmp(&right.intent_id)
                .then_with(|| left.scope_id.cmp(&right.scope_id))
        });
        let connector_error = connectors
            .iter()
            .any(|connector| connector.health == ConnectorHealth::Error);
        let credential_backend = match self.credential_store.backend() {
            else_secrets::CredentialBackend::Keychain => "keychain",
            else_secrets::CredentialBackend::PrivateFile => "private_file",
        }
        .into();
        let retention_days = self.runtime_settings()?.retention_days;
        Ok(RuntimeDiagnosticReport {
            generated_at: Utc::now(),
            runtime_version: env!("CARGO_PKG_VERSION").into(),
            protocol_version: PROTOCOL_VERSION.into(),
            healthy: database_integrity == "ok" && !connector_error,
            database_integrity,
            event_count,
            event_cursor,
            intent_count: specs.len() as u64,
            intent_states,
            connectors,
            scopes,
            storage: RuntimeStorageDiagnostic {
                database_bytes: file_size(&self.settings_path.with_file_name("else.db")),
                workspace_bytes: directory_size(&self.workspace),
                artifact_store_bytes: directory_size(self.store.root()),
                branch_bytes: directory_size(&self.branches),
            },
            credential_backend,
            retention_days,
        })
    }
}

fn state_name(state: IntentState) -> &'static str {
    match state {
        IntentState::Active => "active",
        IntentState::WaitingExternal => "waiting_external",
        IntentState::NeedsDecision => "needs_decision",
        IntentState::Paused => "paused",
        IntentState::Completed => "completed",
        IntentState::Cancelled => "cancelled",
        IntentState::Failed => "failed",
    }
}

fn file_size(path: &Path) -> u64 {
    std::fs::metadata(path)
        .map(|metadata| metadata.len())
        .unwrap_or(0)
}

fn directory_size(path: &Path) -> u64 {
    let Ok(metadata) = std::fs::symlink_metadata(path) else {
        return 0;
    };
    if metadata.file_type().is_symlink() {
        return 0;
    }
    if metadata.is_file() {
        return metadata.len();
    }
    let Ok(entries) = std::fs::read_dir(path) else {
        return 0;
    };
    entries
        .filter_map(|entry| entry.ok())
        .map(|entry| directory_size(&entry.path()))
        .sum()
}
