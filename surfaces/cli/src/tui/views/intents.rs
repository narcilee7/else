//! Intent 列表。

use else_protocol::api::IntentSnapshot;

use super::super::row::{Pane, Row, RowAction};
use super::super::theme;

/// Intent 列表。
pub fn pane(intents: &[IntentSnapshot]) -> Pane {
    let mut rows = Vec::new();
    if intents.is_empty() {
        rows.push(Row::muted("no intents yet — press n to start one"));
        return Pane {
            title: "Intents".into(),
            rows,
        };
    }
    for intent in intents {
        let style = match intent.state {
            else_protocol::intent::IntentState::Completed
            | else_protocol::intent::IntentState::Cancelled => theme::done(),
            else_protocol::intent::IntentState::NeedsDecision => theme::critical(),
            else_protocol::intent::IntentState::Failed => theme::critical(),
            else_protocol::intent::IntentState::WaitingExternal => theme::waiting(),
            _ => theme::text(),
        };
        let project = intent
            .spec
            .project
            .as_deref()
            .map(|project| format!(" [{project}]"))
            .unwrap_or_default();
        rows.push(Row::action(
            format!("  {:?}  {}{project}", intent.state, intent.spec.outcome),
            style,
            RowAction::OpenIntent(intent.spec.id.clone()),
        ));
        rows.push(Row::muted(format!(
            "      {}  artifacts={} decisions={}",
            intent.spec.id,
            intent.artifacts.len(),
            intent.decisions.len()
        )));
    }
    Pane {
        title: "Intents".into(),
        rows,
    }
}
