//! 独立 Outcome Verifier：只检查 Workspace 中的事实，不信任 Harness 自报完成。

use std::path::{Component, Path, PathBuf};

use else_protocol::artifact::OutcomeEvidence;
use else_protocol::intent::{Criterion, CriterionVerification};

pub type Result<T> = std::result::Result<T, String>;

/// 逐项执行机器可检查的 DoneWhen。任何一项失败或无法自动检查，
/// 整次验证失败；不会为未检查的标准生成通过证据。
pub fn verify_outcome(workspace: &Path, criteria: &[Criterion]) -> Result<Vec<OutcomeEvidence>> {
    if criteria.is_empty() {
        return Err("intent has no DoneWhen criteria".into());
    }
    let workspace = workspace
        .canonicalize()
        .map_err(|error| format!("workspace {}: {error}", workspace.display()))?;
    criteria
        .iter()
        .map(|criterion| verify_criterion(&workspace, criterion))
        .collect()
}

fn verify_criterion(workspace: &Path, criterion: &Criterion) -> Result<OutcomeEvidence> {
    let note = match &criterion.verification {
        CriterionVerification::Manual => {
            return Err(format!(
                "criterion requires manual verification: {}",
                criterion.description
            ));
        }
        CriterionVerification::FileExists { path } => {
            let resolved = resolve_existing(workspace, path)?;
            if !resolved.is_file() {
                return Err(format!("expected file but found another object: {path}"));
            }
            format!("verified file exists: {path}")
        }
        CriterionVerification::FileContains { path, text } => {
            let resolved = resolve_existing(workspace, path)?;
            if !resolved.is_file() {
                return Err(format!("expected file but found another object: {path}"));
            }
            let content = std::fs::read_to_string(&resolved)
                .map_err(|error| format!("read {path}: {error}"))?;
            if !content.contains(text) {
                return Err(format!("file {path} does not contain expected text"));
            }
            format!("verified file {path} contains expected text")
        }
    };
    Ok(OutcomeEvidence {
        criterion: criterion.description.clone(),
        artifact_id: None,
        note,
    })
}

fn resolve_existing(workspace: &Path, raw: &str) -> Result<PathBuf> {
    let relative = Path::new(raw);
    if relative.is_absolute()
        || relative
            .components()
            .any(|component| matches!(component, Component::ParentDir | Component::RootDir))
    {
        return Err(format!("criterion path escapes workspace: {raw}"));
    }
    let resolved = workspace
        .join(relative)
        .canonicalize()
        .map_err(|error| format!("missing criterion path {raw}: {error}"))?;
    if !resolved.starts_with(workspace) {
        return Err(format!("criterion path escapes workspace: {raw}"));
    }
    Ok(resolved)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn evaluates_files_without_trusting_process_status() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("result.txt"), "release-ready").unwrap();
        let evidence = verify_outcome(
            dir.path(),
            &[
                Criterion::file_exists("结果存在", "result.txt"),
                Criterion {
                    description: "结果内容正确".into(),
                    verification: CriterionVerification::FileContains {
                        path: "result.txt".into(),
                        text: "release-ready".into(),
                    },
                },
            ],
        )
        .unwrap();
        assert_eq!(evidence.len(), 2);
        assert!(evidence[0].note.contains("exists"));
    }

    #[test]
    fn rejects_missing_manual_and_escaping_criteria() {
        let dir = tempfile::tempdir().unwrap();
        let missing = verify_outcome(dir.path(), &[Criterion::file_exists("缺失", "none.txt")]);
        assert!(missing.unwrap_err().contains("missing"));

        let manual = verify_outcome(
            dir.path(),
            &[Criterion {
                description: "人工判断".into(),
                verification: CriterionVerification::Manual,
            }],
        );
        assert!(manual.unwrap_err().contains("manual"));

        let escaping = verify_outcome(
            dir.path(),
            &[Criterion::file_exists("逃逸", "../outside.txt")],
        );
        assert!(escaping.unwrap_err().contains("escapes workspace"));
    }
}
