//! Results：已经收敛的 Intent 及其结果证据入口。

use else_protocol::api::IntentSnapshot;
use else_protocol::intent::IntentState;

use super::super::row::{Pane, Row, RowAction};
use super::super::theme;
use super::{intent_state_label, intent_style, short_id};

pub fn pane(intents: &[IntentSnapshot]) -> Pane {
    let mut rows = Vec::new();
    let finished: Vec<_> = intents
        .iter()
        .filter(|intent| {
            matches!(
                intent.state,
                IntentState::Completed | IntentState::Cancelled | IntentState::Failed
            )
        })
        .collect();
    if finished.is_empty() {
        rows.push(Row::muted("No finished intents yet."));
        rows.push(Row::muted(
            "Completed outcomes and their evidence will appear here.",
        ));
        return Pane {
            title: "Results".into(),
            rows,
        };
    }
    rows.push(Row::muted(format!(
        "{} finished intent(s) · Enter to inspect the outcome",
        finished.len()
    )));
    for intent in finished {
        let marker = match intent.state {
            IntentState::Completed => "✓",
            IntentState::Failed => "×",
            IntentState::Cancelled => "—",
            _ => unreachable!(),
        };
        rows.push(Row::action(
            format!(
                "  {marker} {}  {}",
                intent_state_label(intent.state),
                intent.spec.outcome
            ),
            intent_style(intent.state),
            RowAction::OpenIntent(intent.spec.id.clone()),
        ));
        rows.push(Row::muted(format!(
            "      {} artifact(s) · {}",
            intent.artifacts.len(),
            short_id(&intent.spec.id)
        )));
    }
    Pane {
        title: "Outcomes".into(),
        rows,
    }
}

pub fn intent_picker(intents: &[IntentSnapshot]) -> Pane {
    let mut rows = Vec::new();
    if intents.is_empty() {
        rows.push(Row::muted("No intents are available."));
        return Pane {
            title: "Choose an intent".into(),
            rows,
        };
    }
    rows.push(Row::muted("Choose which intent's connectors to inspect:"));
    for intent in intents {
        rows.push(Row::action(
            format!("  {}", intent.spec.outcome),
            theme::text(),
            RowAction::OpenConnectors(intent.spec.id.clone()),
        ));
        if let Some(project) = &intent.spec.project {
            rows.push(Row::muted(format!("      {project}")));
        }
    }
    Pane {
        title: "Connector scope".into(),
        rows,
    }
}

pub fn acceptance_rows(intent: &IntentSnapshot) -> Vec<Row> {
    let mut rows = Vec::new();
    rows.push(Row::heading("Acceptance criteria"));
    if intent.spec.done_when.is_empty() {
        rows.push(Row::muted("  No explicit acceptance criteria"));
    } else {
        for criterion in &intent.spec.done_when {
            rows.push(Row::styled(
                format!("  • {}", criterion.description),
                theme::text(),
            ));
        }
    }
    rows.push(Row::heading("Artifacts"));
    if intent.artifacts.is_empty() {
        rows.push(Row::muted("  No artifacts produced yet"));
    } else {
        for artifact in &intent.artifacts {
            let title = if artifact.note.trim().is_empty() {
                artifact.locator.as_str()
            } else {
                artifact.note.as_str()
            };
            rows.push(Row::styled(format!("  • {title}"), theme::text()));
            if !artifact.note.trim().is_empty() && artifact.note != artifact.locator {
                rows.push(Row::muted(format!("      {}", artifact.locator)));
            }
        }
    }
    rows
}
