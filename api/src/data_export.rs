//! User-controlled, credential-free Runtime data export.

use std::path::{Component, Path};

use base64::{engine::general_purpose::STANDARD, Engine as _};
use chrono::{DateTime, Utc};
use else_protocol::artifact::Artifact;
use else_protocol::event::EventEnvelope;
use else_protocol::thread::{ThreadSpec, ThreadState};
use else_protocol::PROTOCOL_VERSION;
use serde::Serialize;

use crate::{Result, Runtime};

#[derive(Debug, Serialize)]
pub struct RuntimeDataExport {
    pub export_version: u32,
    pub protocol_version: String,
    pub exported_at: DateTime<Utc>,
    pub threads: Vec<ThreadDataExport>,
}

#[derive(Debug, Serialize)]
pub struct ThreadDataExport {
    pub spec: ThreadSpec,
    pub state: ThreadState,
    pub cursor: u64,
    pub events: Vec<EventDataExport>,
    pub artifacts: Vec<ArtifactDataExport>,
}

#[derive(Debug, Serialize)]
pub struct EventDataExport {
    pub seq: u64,
    pub envelope: EventEnvelope,
}

#[derive(Debug, Serialize)]
pub struct ArtifactDataExport {
    pub artifact: Artifact,
    pub files: Vec<ExportFile>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unavailable_reason: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ExportFile {
    pub path: String,
    pub size: u64,
    pub content_base64: String,
}

impl Runtime {
    pub fn export_data(&self) -> Result<RuntimeDataExport> {
        let mut threads = Vec::new();
        for spec in self.list_threads()? {
            let projection = self.projection(&spec.id)?;
            // 工件路径相对于该 Thread 的执行位置，不是 Runtime 根目录。
            let workspace_root = self.workspace_of(&projection)?.canonicalize()?;
            let events = self
                .events(&spec.id)?
                .into_iter()
                .map(|event| EventDataExport {
                    seq: event.seq,
                    envelope: event.envelope,
                })
                .collect();
            let artifacts = projection
                .artifacts
                .iter()
                .map(|artifact| self.export_artifact(artifact, &workspace_root))
                .collect();
            threads.push(ThreadDataExport {
                spec,
                state: projection.state.unwrap_or(ThreadState::Active),
                cursor: projection.cursor,
                events,
                artifacts,
            });
        }
        Ok(RuntimeDataExport {
            export_version: 1,
            protocol_version: PROTOCOL_VERSION.into(),
            exported_at: Utc::now(),
            threads,
        })
    }

    fn export_artifact(&self, artifact: &Artifact, workspace_root: &Path) -> ArtifactDataExport {
        let files = if let Some(manifest) = &artifact.manifest {
            match self.store.snapshot_files(manifest) {
                Ok(files) => files
                    .into_iter()
                    .map(|(path, content)| export_file(path, content))
                    .collect(),
                Err(error) => {
                    return ArtifactDataExport {
                        artifact: artifact.clone(),
                        files: vec![],
                        unavailable_reason: Some(error.to_string()),
                    };
                }
            }
        } else {
            match export_locator(&artifact.locator, workspace_root) {
                Ok(files) => files,
                Err(error) => {
                    return ArtifactDataExport {
                        artifact: artifact.clone(),
                        files: vec![],
                        unavailable_reason: Some(error),
                    };
                }
            }
        };
        ArtifactDataExport {
            artifact: artifact.clone(),
            files,
            unavailable_reason: None,
        }
    }
}

fn export_locator(
    locator: &str,
    workspace_root: &Path,
) -> std::result::Result<Vec<ExportFile>, String> {
    let path = Path::new(locator);
    if path.is_absolute() || !path.is_relative() {
        return Err("artifact locator is not relative to the workspace".into());
    }
    if is_private_path(path) {
        return Err("artifact locator points to private runtime state".into());
    }
    let source = workspace_root.join(path);
    let canonical = source.canonicalize().map_err(|error| error.to_string())?;
    if !canonical.starts_with(workspace_root) {
        return Err("artifact locator escapes the workspace".into());
    }
    let mut files = Vec::new();
    collect_files(&canonical, workspace_root, &mut files)?;
    Ok(files)
}

fn collect_files(
    path: &Path,
    workspace_root: &Path,
    output: &mut Vec<ExportFile>,
) -> std::result::Result<(), String> {
    if path.is_file() {
        let relative = path
            .strip_prefix(workspace_root)
            .map_err(|error| error.to_string())?
            .to_string_lossy()
            .to_string();
        if !is_private_path(Path::new(&relative)) {
            let content = std::fs::read(path).map_err(|error| error.to_string())?;
            output.push(export_file(relative, content));
        }
        return Ok(());
    }
    for entry in std::fs::read_dir(path).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        collect_files(&entry.path(), workspace_root, output)?;
    }
    output.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(())
}

fn export_file(path: String, content: Vec<u8>) -> ExportFile {
    ExportFile {
        path,
        size: content.len() as u64,
        content_base64: STANDARD.encode(content),
    }
}

fn is_private_path(path: &Path) -> bool {
    path.components().any(|component| {
        let Component::Normal(name) = component else {
            return false;
        };
        matches!(
            name.to_str(),
            Some(".else-harness" | ".codex" | ".claude" | "harness-private")
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locator_export_rejects_paths_outside_workspace() {
        let parent = tempfile::tempdir().unwrap();
        let workspace = parent.path().join("workspace");
        std::fs::create_dir(&workspace).unwrap();
        std::fs::write(parent.path().join("outside.txt"), "outside-secret").unwrap();

        let error = export_locator("../outside.txt", &workspace).unwrap_err();
        assert!(error.contains("escapes") || error.contains("relative"));
    }
}
