//! 独立 Outcome Verifier：只检查 Workspace 中的事实，不信任 Harness 自报完成。

use std::path::{Component, Path, PathBuf};

use else_protocol::artifact::OutcomeEvidence;
use else_protocol::thread::{Criterion, CriterionVerification};

pub type Result<T> = std::result::Result<T, String>;

/// 一次验收评估的结果。Manual 标准不能伪造通过，只能转为用户确认。
#[derive(Debug, Clone, PartialEq)]
pub enum OutcomeVerification {
    Verified(Vec<OutcomeEvidence>),
    NeedsConfirmation {
        evidence: Vec<OutcomeEvidence>,
        criteria: Vec<Criterion>,
    },
}

/// 逐项执行机器可检查的 DoneWhen。任何一项失败或无法自动检查，
/// 会返回待人工确认，不会为未检查的标准生成通过证据。
pub fn evaluate_outcome(workspace: &Path, criteria: &[Criterion]) -> Result<OutcomeVerification> {
    if criteria.is_empty() {
        return Err("thread has no DoneWhen criteria".into());
    }
    let workspace = workspace
        .canonicalize()
        .map_err(|error| format!("workspace {}: {error}", workspace.display()))?;
    let mut evidence = Vec::new();
    let mut manual = Vec::new();
    for criterion in criteria {
        if criterion.verification == CriterionVerification::Manual {
            manual.push(criterion.clone());
        } else {
            evidence.push(verify_criterion(&workspace, criterion)?);
        }
    }
    if manual.is_empty() {
        Ok(OutcomeVerification::Verified(evidence))
    } else {
        Ok(OutcomeVerification::NeedsConfirmation {
            evidence,
            criteria: manual,
        })
    }
}

/// 严格模式：存在 Manual 标准时失败关闭。
pub fn verify_outcome(workspace: &Path, criteria: &[Criterion]) -> Result<Vec<OutcomeEvidence>> {
    match evaluate_outcome(workspace, criteria)? {
        OutcomeVerification::Verified(evidence) => Ok(evidence),
        OutcomeVerification::NeedsConfirmation { .. } => {
            Err("criterion requires manual verification".into())
        }
    }
}

/// 用户批准后重新检查所有机器标准，并将 Manual 标准记录为用户确认。
pub fn confirm_manual_outcome(
    workspace: &Path,
    criteria: &[Criterion],
) -> Result<Vec<OutcomeEvidence>> {
    if criteria.is_empty() {
        return Err("thread has no DoneWhen criteria".into());
    }
    let workspace = workspace
        .canonicalize()
        .map_err(|error| format!("workspace {}: {error}", workspace.display()))?;
    criteria
        .iter()
        .map(|criterion| match &criterion.verification {
            CriterionVerification::Manual => Ok(OutcomeEvidence {
                criterion: criterion.description.clone(),
                artifact_id: None,
                note: "verified by explicit user confirmation".into(),
            }),
            _ => verify_criterion(&workspace, criterion),
        })
        .collect()
}

fn verify_criterion(workspace: &Path, criterion: &Criterion) -> Result<OutcomeEvidence> {
    let note = match &criterion.verification {
        CriterionVerification::Manual => {
            return Err("manual criterion reached strict verifier".into())
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

    #[test]
    fn manual_criteria_require_confirmation_without_faking_evidence() {
        let dir = tempfile::tempdir().unwrap();
        let criteria = [
            Criterion::file_exists("产物存在", "result.txt"),
            Criterion::manual("结论符合业务预期"),
        ];
        std::fs::write(dir.path().join("result.txt"), "ok").unwrap();
        let pending = evaluate_outcome(dir.path(), &criteria).unwrap();
        assert!(matches!(
            pending,
            OutcomeVerification::NeedsConfirmation { criteria, .. } if criteria.len() == 1
        ));
        let confirmed = confirm_manual_outcome(dir.path(), &criteria).unwrap();
        assert_eq!(confirmed.len(), 2);
        assert!(confirmed[1].note.contains("user confirmation"));
    }
}
