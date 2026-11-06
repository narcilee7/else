//! 全部 Thread：首屏每组只显示三行，这里是完整列表。
//!
//! 它存在的唯一理由是"首屏不堆东西"。分组按状态排：还在动的在上，收尾的在下。

use else_protocol::api::ThreadSnapshot;
use else_protocol::thread::ThreadState;

use super::super::row::{Row, RowAction};
use super::super::theme;
use super::{short_id, thread_state_label, thread_style};

pub fn rows(threads: &[ThreadSnapshot]) -> Vec<Row> {
    if threads.is_empty() {
        return vec![Row::blank(), Row::muted(" No thread has been opened yet.")];
    }

    let mut ordered: Vec<&ThreadSnapshot> = threads.iter().collect();
    ordered.sort_by_key(|thread| rank(thread.state));

    let mut rows = vec![Row::heading(" ALL THREADS")];
    for thread in ordered {
        rows.push(
            Row::action(
                format!(" {} {}", marker(thread.state), thread.spec.outcome),
                thread_style(thread.state),
                RowAction::OpenThread(thread.spec.id.clone()),
            )
            .with_meta(format!(
                "{} · {} tokens · {}",
                thread_state_label(thread.state),
                thread.resource_usage.model_tokens,
                short_id(&thread.spec.id)
            ))
            .with_note(thread_note(thread)),
        );
    }
    rows
}

/// Connectors 屏幕要知道看哪条 Thread 的来源。
pub fn thread_picker_rows(threads: &[ThreadSnapshot]) -> Vec<Row> {
    if threads.is_empty() {
        return vec![Row::muted(" No thread is available.")];
    }
    let mut rows = vec![Row::heading(" WHICH THREAD")];
    for thread in threads {
        rows.push(
            Row::action(
                format!(" {}", thread.spec.outcome),
                theme::text(),
                RowAction::OpenConnectors(thread.spec.id.clone()),
            )
            .with_meta(format!("{} artifacts", thread.artifacts.len())),
        );
    }
    rows
}

fn thread_note(thread: &ThreadSnapshot) -> String {
    let mut parts = vec![format!("mode {}", thread.spec.mode.label())];
    if let Some(project) = thread.spec.project.as_deref() {
        parts.push(format!("project {project}"));
    }
    parts.push(format!("revision {}", thread.spec.revision));
    parts.join(" · ")
}

fn rank(state: ThreadState) -> u8 {
    match state {
        ThreadState::NeedsDecision => 0,
        ThreadState::Active => 1,
        ThreadState::WaitingExternal => 2,
        ThreadState::Paused => 3,
        ThreadState::Failed => 4,
        ThreadState::Completed => 5,
        ThreadState::Cancelled => 6,
    }
}

fn marker(state: ThreadState) -> &'static str {
    match state {
        ThreadState::Completed => "✓",
        ThreadState::Failed => "×",
        ThreadState::Cancelled => "—",
        ThreadState::NeedsDecision => "▸",
        ThreadState::Paused => "⏸",
        ThreadState::WaitingExternal => "…",
        ThreadState::Active => "●",
    }
}
