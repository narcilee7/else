//! Thread 详情：默认展示目标、下一步、计划与证据；调试信息显式展开。

use else_protocol::plan::PlanStepStatus;

use super::super::data::ThreadDetailData;
use super::super::row::{Pane, Row, RowAction};
use super::super::theme;
use super::{results, short_id, thread_state_label, thread_style};

pub fn pane(detail: &ThreadDetailData, debug: bool) -> Pane {
    let mut rows = Vec::new();
    let thread = &detail.thread;

    rows.push(Row::styled(
        format!("  {}", thread.spec.outcome),
        theme::heading(),
    ));
    rows.push(Row::styled(
        format!("  {}", thread_state_label(thread.state)),
        thread_style(thread.state),
    ));
    if let Some(project) = thread.spec.project.as_deref() {
        rows.push(Row::muted(format!("  Project · {project}")));
    }
    if !thread.spec.constraints.is_empty() {
        rows.push(Row::heading("Boundaries"));
        for constraint in &thread.spec.constraints {
            rows.push(Row::muted(format!("  • {constraint}")));
        }
    }

    if let Some(request) = thread.decisions.first() {
        rows.push(Row::heading("Next step"));
        rows.push(Row::action(
            format!("  Review · {}", request.summary),
            theme::critical(),
            RowAction::OpenDecision {
                thread_id: thread.spec.id.clone(),
                request_id: request.id.clone(),
            },
        ));
    } else if !matches!(
        thread.state,
        else_protocol::thread::ThreadState::Completed
            | else_protocol::thread::ThreadState::Cancelled
            | else_protocol::thread::ThreadState::Failed
    ) {
        rows.push(Row::heading("Next step"));
        rows.push(Row::action(
            "  Advance this thread",
            theme::focused(),
            RowAction::AdvanceThread(thread.spec.id.clone()),
        ));
    }

    rows.extend(results::acceptance_rows(thread));

    rows.push(Row::heading("Plan"));
    match detail.plan.current.as_ref() {
        Some(plan) => {
            rows.push(Row::styled(format!("  {}", plan.summary), theme::text()));
            for step in &plan.steps {
                rows.push(Row::muted(format!(
                    "  {}  {}",
                    step_status_label(step.status),
                    step.title
                )));
            }
        }
        None => rows.push(Row::muted("  No plan has been produced yet")),
    }

    if debug {
        rows.push(Row::heading("Debug · Scopes"));
        if detail.scopes.is_empty() {
            rows.push(Row::muted("  No scopes"));
        }
        for scope in &detail.scopes {
            rows.push(Row::muted(format!(
                "  {}  {:?}  generation={} failures={}",
                scope.scope_id, scope.state, scope.generation, scope.failure_count
            )));
            rows.push(Row::muted(format!("      {}", scope.outcome)));
        }
        rows.push(Row::heading("Debug · Resources"));
        rows.push(Row::muted(format!(
            "  tokens {}/{} · tool calls {}/{} · sandbox {}s/{}s",
            detail.resources.usage.model_tokens,
            detail.resources.budget.max_model_tokens,
            detail.resources.usage.tool_calls,
            detail.resources.budget.max_tool_calls,
            detail.resources.usage.sandbox_seconds,
            detail.resources.budget.max_sandbox_seconds
        )));
        rows.push(Row::heading("Debug · Events"));
        let start = detail.events.len().saturating_sub(12);
        if detail.events.is_empty() {
            rows.push(Row::muted("  No events"));
        }
        for event in &detail.events[start..] {
            rows.push(Row::muted(format!(
                "  #{} {} {}",
                event.seq, event.scope_id, event.event_id
            )));
        }
        rows.push(Row::muted(format!(
            "  thread={} revision={} cursor={}",
            short_id(&thread.spec.id),
            thread.spec.revision,
            thread.cursor
        )));
    } else {
        rows.push(Row::muted(
            "  d  show scopes, resources and event debug details",
        ));
    }

    Pane {
        title: format!("Thread · {}", short_id(&thread.spec.id)),
        rows,
    }
}

fn step_status_label(status: PlanStepStatus) -> &'static str {
    match status {
        PlanStepStatus::Planned => "○",
        PlanStepStatus::Ready => "→",
        PlanStepStatus::Running => "…",
        PlanStepStatus::Completed => "✓",
        PlanStepStatus::NeedsReplan => "!",
        PlanStepStatus::Blocked => "×",
        PlanStepStatus::Superseded => "—",
        PlanStepStatus::Cancelled => "—",
    }
}
