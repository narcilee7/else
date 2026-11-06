//! Persistent runtime-level policy settings.

use std::io::Write;
use std::path::Path;

use chrono::{DateTime, Duration, Utc};
use else_protocol::event::EventPayload;
use else_protocol::thread::ThreadState;

use crate::{ApiError, Result, Runtime, ThreadDeletionReport};

pub use else_protocol::runtime::RuntimeSettings;

impl Runtime {
    pub fn runtime_settings(&self) -> Result<RuntimeSettings> {
        let data = match std::fs::read(&self.settings_path) {
            Ok(data) => data,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(RuntimeSettings::default())
            }
            Err(error) => return Err(error.into()),
        };
        serde_json::from_slice(&data)
            .map_err(|error| ApiError::Conflict(format!("runtime settings: {error}")))
    }

    pub fn update_runtime_settings(&self, settings: RuntimeSettings) -> Result<RuntimeSettings> {
        if settings
            .retention_days
            .is_some_and(|days| days == 0 || days > 3_650)
        {
            return Err(ApiError::Conflict(
                "retention_days must be between 1 and 3650".into(),
            ));
        }
        write_private_json(&self.settings_path, &settings)?;
        Ok(settings)
    }

    pub fn enforce_retention(&self, now: DateTime<Utc>) -> Result<Vec<ThreadDeletionReport>> {
        let Some(days) = self.runtime_settings()?.retention_days else {
            return Ok(vec![]);
        };
        let cutoff = now - Duration::days(days as i64);
        let mut deleted = Vec::new();
        for spec in self.list_threads()? {
            let projection = self.projection(&spec.id)?;
            if !matches!(
                projection.state,
                Some(ThreadState::Completed | ThreadState::Failed | ThreadState::Cancelled)
            ) {
                continue;
            }
            let terminal_at = self.events(&spec.id)?.iter().rev().find_map(|event| {
                match &event.envelope.payload {
                    EventPayload::ThreadStateChanged { state }
                        if matches!(
                            *state,
                            ThreadState::Completed | ThreadState::Failed | ThreadState::Cancelled
                        ) =>
                    {
                        Some(event.envelope.occurred_at)
                    }
                    _ => None,
                }
            });
            if terminal_at.is_some_and(|terminal_at| terminal_at <= cutoff) {
                deleted.push(self.delete_thread(&spec.id)?);
            }
        }
        Ok(deleted)
    }
}

fn write_private_json(path: &Path, settings: &RuntimeSettings) -> Result<()> {
    let data = serde_json::to_vec_pretty(settings)
        .map_err(|error| ApiError::Conflict(format!("runtime settings: {error}")))?;
    let temporary = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
    let mut options = std::fs::OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let result = (|| {
        let mut file = options.open(&temporary)?;
        file.write_all(&data)?;
        file.sync_all()?;
        std::fs::rename(&temporary, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}
