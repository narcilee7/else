//! Now：现在什么在等我。
//!
//! 四个分组，按"该不该你管"排序：需要你 → 在跑 → 刚完 → 世界。
//! 每组最多三行，超出的部分不往屏幕里堆，而是告诉你按哪个键。
//! 每一行一行说完：左边是什么事，右边是判断它要的信息。

use else_protocol::api::ThreadSnapshot;
use else_protocol::attention::AttentionRequest;
use else_protocol::ids::ThreadId;
use else_protocol::thread::ThreadState;
use else_protocol::world::WorldItemRecord;

use super::super::data::OverviewData;
use super::super::row::{Row, RowAction};
use super::super::theme;
use super::{loss_label, loss_style, thread_style};

/// 每组最多显示多少行。超过的部分只报数量。
const NEEDS_YOU_LIMIT: usize = 3;
const RUNNING_LIMIT: usize = 3;
const RECENT_LIMIT: usize = 3;
const WORLD_LIMIT: usize = 2;

pub fn rows(data: &OverviewData) -> Vec<Row> {
    let mut rows = Vec::new();
    let requests = decision_requests(data);
    let running: Vec<&ThreadSnapshot> = data
        .threads
        .iter()
        .filter(|thread| {
            matches!(
                thread.state,
                ThreadState::Active | ThreadState::WaitingExternal | ThreadState::Paused
            )
        })
        .collect();
    let finished: Vec<&ThreadSnapshot> = data
        .threads
        .iter()
        .filter(|thread| {
            matches!(
                thread.state,
                ThreadState::Completed | ThreadState::Cancelled | ThreadState::Failed
            )
        })
        .collect();

    if requests.is_empty() && data.threads.is_empty() {
        return empty_state();
    }

    push_needs_you(&mut rows, &requests);
    push_running(&mut rows, &running);
    push_recent(&mut rows, &finished);
    push_world(&mut rows, data);
    rows
}

/// 首屏没有数据时，不给空表格，给下一个具体动作。
fn empty_state() -> Vec<Row> {
    vec![
        Row::blank(),
        Row::styled(" Nothing is running yet.", theme::text()),
        Row::blank(),
        Row::muted(" Press n and describe an outcome you want handled."),
        Row::muted(" else reports where it works, what it spends, and when it needs you."),
    ]
}

fn push_needs_you(rows: &mut Vec<Row>, requests: &[(ThreadId, AttentionRequest)]) {
    if requests.is_empty() {
        return;
    }
    rows.push(Row::heading(" NEEDS YOU"));
    for (thread_id, request) in requests.iter().take(NEEDS_YOU_LIMIT) {
        rows.push(
            Row::action(
                format!(" ▸ {}", request.summary),
                loss_style(request.expected_loss),
                RowAction::OpenThread(thread_id.clone()),
            )
            .with_meta(format!(
                "{} · {thread_id}",
                loss_label(request.expected_loss)
            ))
            .with_note(request.blocked_reason.clone()),
        );
    }
    if requests.len() > NEEDS_YOU_LIMIT {
        rows.push(Row::muted(format!(
            " +{} more · d reviews all of them",
            requests.len() - NEEDS_YOU_LIMIT
        )));
    }
    rows.push(Row::blank());
}

fn push_running(rows: &mut Vec<Row>, running: &[&ThreadSnapshot]) {
    if running.is_empty() {
        return;
    }
    rows.push(Row::heading(" RUNNING"));
    for thread in running.iter().take(RUNNING_LIMIT) {
        rows.push(
            Row::action(
                format!(" ● {}", thread.spec.outcome),
                thread_style(thread.state),
                RowAction::OpenThread(thread.spec.id.clone()),
            )
            .with_meta(thread_meta(thread)),
        );
    }
    if running.len() > RUNNING_LIMIT {
        rows.push(more_row(running.len() - RUNNING_LIMIT));
    }
    rows.push(Row::blank());
}

fn push_recent(rows: &mut Vec<Row>, finished: &[&ThreadSnapshot]) {
    if finished.is_empty() {
        return;
    }
    rows.push(Row::heading(" RECENT"));
    for thread in finished.iter().take(RECENT_LIMIT) {
        rows.push(
            Row::action(
                format!(" ✓ {}", thread.spec.outcome),
                thread_style(thread.state),
                RowAction::OpenThread(thread.spec.id.clone()),
            )
            .with_meta(format!(
                "{} artifacts · {}",
                thread.artifacts.len(),
                super::thread_state_label(thread.state)
            )),
        );
    }
    if finished.len() > RECENT_LIMIT {
        rows.push(more_row(finished.len() - RECENT_LIMIT));
    }
    rows.push(Row::blank());
}

/// "还有几条"不应该是一句死文字：它本身就是一个能按的东西。
fn more_row(rest: usize) -> Row {
    Row::action(
        format!(" +{rest} more"),
        theme::muted(),
        RowAction::OpenResults,
    )
    .with_note("⏎ shows every thread")
}

/// 世界里的承诺、等待与到期。只有真的有外部来源时才占空间。
fn push_world(rows: &mut Vec<Row>, data: &OverviewData) {
    let mut pushed = false;
    let mut push = |records: &[WorldItemRecord]| {
        for record in records.iter().take(WORLD_LIMIT) {
            if !pushed {
                rows.push(Row::heading(" WORLD"));
                pushed = true;
            }
            rows.push(
                Row::action(
                    format!(" · {}", record.item.title),
                    theme::text(),
                    RowAction::ResolveWorld {
                        thread_id: record.thread_id.clone(),
                        item_id: record.item.id.clone(),
                        kind: else_protocol::world::WorldItemResolutionKind::Completed,
                    },
                )
                .with_meta(record_meta(record))
                .with_note("⏎ marks this item done"),
            );
        }
    };
    push(&data.brief.overdue);
    push(&data.brief.waiting_external);
    push(&data.brief.open_commitments);
    push(&data.brief.action_items_due_soon);
    if pushed {
        rows.push(Row::blank());
    }
}

fn record_meta(record: &WorldItemRecord) -> String {
    if let Some(deadline) = record.item.deadline {
        return format!("due {}", deadline.format("%m-%d %H:%M"));
    }
    format!("{:?}", record.item.state).to_lowercase()
}

/// 一条 Thread 的判断依据：在哪做、多久、花多少。
fn thread_meta(thread: &ThreadSnapshot) -> String {
    let mut parts = vec![thread.spec.mode.label().to_string()];
    if let Some(project) = thread.spec.project.as_deref() {
        parts.push(project.to_string());
    }
    if thread.resource_usage.model_tokens > 0 {
        parts.push(format!("{} tok", thread.resource_usage.model_tokens));
    }
    parts.join(" · ")
}

fn decision_requests(data: &OverviewData) -> Vec<(ThreadId, AttentionRequest)> {
    let mut requests = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for window in &data.attention {
        for request in &window.requests {
            if seen.insert(request.id.clone()) {
                requests.push((window.thread_id.clone(), request.clone()));
            }
        }
    }
    for thread in &data.threads {
        for request in &thread.decisions {
            if seen.insert(request.id.clone()) {
                requests.push((thread.spec.id.clone(), request.clone()));
            }
        }
    }
    requests
}
