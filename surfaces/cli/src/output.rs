//! CLI 输出。
//!
//! 呈现的是**契约类型**（`else_protocol::api` 里的形状），不是 Runtime 的
//! 内部投影。CLI / TUI / App 读的是同一份数据，因此也应当对同一件事
//! 给出同一个解释。

use else_protocol::api::{AttentionView, AttentionWindowSnapshot, PlanView, ThreadSnapshot};
use else_protocol::attention::AttentionRequest;
use else_protocol::connector::{ConnectorAction, ThreadCandidate};
use else_protocol::thread::ThreadState;
use else_protocol::world::{WorldItem, WorldItemRecord};

/// Thread 状态的人话说法。
///
/// CLI 和 TUI 说的是同一件事，就该用同一个词 —— 否则 `state: NeedsDecision`
/// 和界面上的 `needs you` 会让人以为是两个状态。
pub fn thread_state_label(state: ThreadState) -> &'static str {
    match state {
        ThreadState::Active => "running",
        ThreadState::WaitingExternal => "waiting",
        ThreadState::NeedsDecision => "needs you",
        ThreadState::Paused => "paused",
        ThreadState::Completed => "done",
        ThreadState::Cancelled => "cancelled",
        ThreadState::Failed => "failed",
    }
}

/// 已经收尾、不会自己再动的状态。
pub fn is_terminal(state: ThreadState) -> bool {
    matches!(
        state,
        ThreadState::Completed | ThreadState::Cancelled | ThreadState::Failed
    )
}

/// 打印一个 Thread 快照。
pub fn print_snapshot(snapshot: &ThreadSnapshot) {
    println!("goal: {}", snapshot.spec.outcome);
    println!("state: {}", thread_state_label(snapshot.state));
    println!("artifacts: {}", snapshot.artifacts.len());
    print_decisions(&[], &snapshot.decisions);
}

/// 打印决策队列。
///
/// `windows` 是可延后分组的窗口，`requests` 是当前需要裁决的请求。
/// 已经在窗口里的请求不会重复列出 —— 用户看到的是「一件事要做一次」。
pub fn print_decisions(windows: &[AttentionWindowSnapshot], requests: &[AttentionRequest]) {
    let mut grouped = std::collections::BTreeSet::new();
    for window in windows {
        let label = match window.window.kind {
            else_protocol::attention::AttentionWindowKind::Batch => "batch",
            else_protocol::attention::AttentionWindowKind::Defer => "deferred",
        };
        println!(
            "decision window {} ({label}, {} items):",
            window.window.id,
            window.requests.len()
        );
        for request in &window.requests {
            grouped.insert(request.id.clone());
        }
        for request in &window.requests {
            print_decision_request(request);
        }
    }
    let ungrouped: Vec<_> = requests
        .iter()
        .filter(|request| !grouped.contains(&request.id))
        .collect();
    if ungrouped.is_empty() && grouped.is_empty() {
        println!("decisions: none");
        return;
    }
    for request in ungrouped {
        print_decision_request(request);
    }
}

fn print_decision_request(request: &AttentionRequest) {
    println!("decision {}: {}", request.id, request.summary);
    for option in &request.options {
        println!("  [{}] {}", option.id, option.label);
    }
}

/// 打印注意力成本账。
pub fn print_attention(view: &AttentionView) {
    let metrics = &view.metrics;
    println!(
        "attention: requested={} interventions={} resolved={}",
        metrics.requested, metrics.interventions, metrics.resolved
    );
    println!(
        "signals: interrupt={} batch={} defer={} silent={}",
        metrics.interrupt_notifications,
        metrics.batch_notifications,
        metrics.defer_notifications,
        metrics.silent_requests
    );
    println!(
        "quality: batch_rate={} misinterrupt_rate={} missed_critical={}",
        format_rate(metrics.batch_rate),
        format_rate(metrics.misinterrupt_rate),
        metrics.missed_critical
    );
    if view.windows.is_empty() {
        println!("windows: none");
        return;
    }
    for window in &view.windows {
        println!(
            "window {} kind={:?} members={} due={} closed={}",
            window.id,
            window.kind,
            window.request_ids.len(),
            window.due_at.to_rfc3339(),
            window
                .closed_at
                .map(|at| at.to_rfc3339())
                .unwrap_or_else(|| "open".into())
        );
    }
}

fn format_rate(rate: Option<f64>) -> String {
    match rate {
        Some(rate) => format!("{:.1}%", rate * 100.0),
        None => "n/a".into(),
    }
}

/// 打印当前计划、版本历史与影响分析。
pub fn print_plan(view: &PlanView) {
    let Some(plan) = view.current.as_ref() else {
        println!("plan: none");
        return;
    };
    println!(
        "plan {} revision={} thread_revision={} supersedes={:?}",
        plan.id, plan.revision, plan.thread_revision, plan.supersedes
    );
    println!("summary: {}", plan.summary);
    for step in &plan.steps {
        println!(
            "step {} status={:?} scope={:?}: {}",
            step.id, step.status, step.scope_id, step.title
        );
        if !step.watch_sources.is_empty() || !step.watch_terms.is_empty() {
            println!(
                "  watches: sources={:?} terms={:?}",
                step.watch_sources, step.watch_terms
            );
        }
    }
    for evidence in &plan.evidence {
        println!(
            "evidence {:?} {}: {}",
            evidence.kind,
            evidence.event_id.as_deref().unwrap_or("-"),
            evidence.summary
        );
    }
    if !view.impacts.is_empty() {
        println!("impacts:");
        for impact in &view.impacts {
            println!(
                "  {} -> affected={} scopes={} unaffected={}",
                impact.source,
                impact.affected_step_ids.len(),
                impact.invalidated_scope_ids.len(),
                impact.unaffected_step_ids.len()
            );
            println!("  {}", impact.explanation);
        }
    }
}

/// 打印一个 Thread 候选。
pub fn print_candidate(candidate: &ThreadCandidate) {
    println!(
        "{} {:?} {:?} confidence={} signals={} outcome={}",
        candidate.id,
        candidate.origin,
        candidate.state,
        candidate.confidence,
        candidate.related_signal_ids.len(),
        candidate.proposed_outcome
    );
}

/// 打印一组 World 条目。
pub fn print_world_section(name: &str, records: &[WorldItemRecord]) {
    println!("{name}: {}", records.len());
    for record in records {
        println!(
            "  thread={} {} {:?} {:?} due={} {}",
            record.thread_id,
            record.item.id,
            record.item.kind,
            record.item.state,
            record
                .item
                .deadline
                .map(|value| value.to_rfc3339())
                .unwrap_or_else(|| "none".into()),
            record.item.title
        );
    }
}

/// 打印一个 World 条目。
pub fn print_world_item(item: &WorldItem) {
    println!(
        "{} {:?} {:?} due={} signals={} connectors={} {}",
        item.id,
        item.kind,
        item.state,
        item.deadline
            .map(|value| value.to_rfc3339())
            .unwrap_or_else(|| "none".into()),
        item.signal_ids.len(),
        item.connector_ids.len(),
        item.title
    );
}

/// 打印一个外部动作及其最终效果。
pub fn print_connector_action(action: &ConnectorAction) {
    println!(
        "{} connector={} capability={} target={} status={:?} effect={:?} attention={} outcome={}",
        action.id,
        action.connector_id,
        action.capability,
        action.target,
        action.status,
        action.effect,
        action
            .attention_id
            .as_ref()
            .map(ToString::to_string)
            .unwrap_or_else(|| "none".into()),
        action
            .outcome
            .as_ref()
            .map(|outcome| format!("{:?}/{:?}", outcome.status, outcome.effect))
            .unwrap_or_else(|| "none".into())
    );
    if let Some(error) = &action.last_error {
        println!("  error={error}");
    }
}
