//! Results：已经收敛的 Thread 及其结果证据入口。

use else_protocol::api::ThreadSnapshot;
use else_protocol::thread::ThreadState;

use super::super::row::{Pane, Row, RowAction};
use super::super::theme;
use super::{short_id, thread_state_label, thread_style};

pub fn pane(threads: &[ThreadSnapshot]) -> Pane {
    let mut rows = Vec::new();
    let finished: Vec<_> = threads
        .iter()
        .filter(|thread| {
            matches!(
                thread.state,
                ThreadState::Completed | ThreadState::Cancelled | ThreadState::Failed
            )
        })
        .collect();
    if finished.is_empty() {
        rows.push(Row::muted("No finished threads yet."));
        rows.push(Row::muted(
            "Completed outcomes and their evidence will appear here.",
        ));
        return Pane {
            title: "Results".into(),
            rows,
        };
    }
    rows.push(Row::muted(format!(
        "{} finished thread(s) · Enter to inspect the outcome",
        finished.len()
    )));
    for thread in finished {
        let marker = match thread.state {
            ThreadState::Completed => "✓",
            ThreadState::Failed => "×",
            ThreadState::Cancelled => "—",
            _ => unreachable!(),
        };
        rows.push(Row::action(
            format!(
                "  {marker} {}  {}",
                thread_state_label(thread.state),
                thread.spec.outcome
            ),
            thread_style(thread.state),
            RowAction::OpenThread(thread.spec.id.clone()),
        ));
        rows.push(Row::muted(format!(
            "      {} artifact(s) · {}",
            thread.artifacts.len(),
            short_id(&thread.spec.id)
        )));
    }
    Pane {
        title: "Outcomes".into(),
        rows,
    }
}

pub fn thread_picker(threads: &[ThreadSnapshot]) -> Pane {
    let mut rows = Vec::new();
    if threads.is_empty() {
        rows.push(Row::muted("No threads are available."));
        return Pane {
            title: "Choose an thread".into(),
            rows,
        };
    }
    rows.push(Row::muted("Choose which thread's connectors to inspect:"));
    for thread in threads {
        rows.push(Row::action(
            format!("  {}", thread.spec.outcome),
            theme::text(),
            RowAction::OpenConnectors(thread.spec.id.clone()),
        ));
        if let Some(project) = &thread.spec.project {
            rows.push(Row::muted(format!("      {project}")));
        }
    }
    Pane {
        title: "Connector scope".into(),
        rows,
    }
}

pub fn acceptance_rows(thread: &ThreadSnapshot) -> Vec<Row> {
    let mut rows = Vec::new();
    rows.push(Row::heading("Acceptance criteria"));
    if thread.spec.done_when.is_empty() {
        rows.push(Row::muted("  No explicit acceptance criteria"));
    } else {
        for criterion in &thread.spec.done_when {
            rows.push(Row::styled(
                format!("  • {}", criterion.description),
                theme::text(),
            ));
        }
    }
    rows.push(Row::heading("Artifacts"));
    if thread.artifacts.is_empty() {
        rows.push(Row::muted("  No artifacts produced yet"));
    } else {
        for artifact in &thread.artifacts {
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
