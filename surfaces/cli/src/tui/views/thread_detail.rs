//! Thread：这件事到底怎么样了。
//!
//! 这一屏是一段时间线，不是一张状态表。顶部三行是事实，中间是 agent 真的
//! 做过什么（由账本事件投影而来），该你决定的事以时间线上最后一行的形式
//! 出现，选项就在它下面 —— 判断和决定之间不切屏。
//!
//! 账本里的事件类型远多于这里出现的东西：界面只投影用户要读的那几种，
//! 其余留在 `d` 之后。

use else_protocol::api::{EventResponse, ThreadSnapshot};
use else_protocol::attention::DecisionChoice;
use else_protocol::plan::PlanStepStatus;
use ratatui::style::Style;

use super::super::data::ThreadDetailData;
use super::super::row::{Row, RowAction};
use super::super::theme;
use super::short_id;

/// 单条 agent 输出最多占几行；剩下的折进 note。
const OUTPUT_LINES: usize = 12;

pub fn rows(detail: &ThreadDetailData, debug: bool) -> Vec<Row> {
    let mut rows = Vec::new();
    let thread = &detail.thread;

    rows.extend(facts(thread));
    rows.extend(needs_you(thread));
    // 还挂着的那条请求在上面等着回答，时间线里就别重复念一遍它的正文。
    let pending: Vec<String> = thread
        .decisions
        .iter()
        .map(|request| request.id.to_string())
        .collect();
    rows.extend(timeline(&detail.events, &pending));
    rows.extend(plan(detail));
    rows.extend(outcome(thread));
    if debug {
        rows.extend(debug_rows(detail));
    }
    rows
}

/// 顶部事实：目标、在哪做、什么时候到期。
fn facts(thread: &ThreadSnapshot) -> Vec<Row> {
    let mut rows = vec![
        Row::styled(format!(" {}", thread.spec.outcome), theme::heading()),
        Row::blank(),
    ];
    let mut facts = vec![format!("works in {}", thread.spec.mode.label())];
    if let Some(project) = thread.spec.project.as_deref() {
        facts.push(format!("project {project}"));
    }
    if let Some(deadline) = thread.spec.deadline {
        facts.push(format!("due {}", deadline.format("%m-%d %H:%M")));
    }
    facts.push(format!("revision {}", thread.spec.revision));
    rows.push(Row::muted(format!(" {}", facts.join(" · "))));
    if !thread.spec.constraints.is_empty() {
        rows.push(Row::muted(format!(
            " limits {}",
            thread.spec.constraints.join(" · ")
        )));
    }
    rows.push(Row::blank());
    rows
}

/// 现在等你决定的事。选项就是行，选中即生效。
///
/// 这里比首屏多一行"为什么它不能自己决定"：首屏是扫一眼的列表，这里是
/// 真的要下判断的地方，理由不能藏进需要先选中的地方。
fn needs_you(thread: &ThreadSnapshot) -> Vec<Row> {
    let mut rows = Vec::new();
    for request in &thread.decisions {
        rows.push(
            Row::styled(
                format!(" ⚠ needs you · {}", request.summary),
                theme::critical(),
            )
            .with_meta(format!(
                "{} impact",
                super::loss_label(request.expected_loss)
            )),
        );
        rows.push(Row::muted(format!(
            "   {} · {}",
            request.blocked_reason,
            if request.reversible {
                "waiting is reversible"
            } else {
                "waiting is not reversible"
            }
        )));
        for option in &request.options {
            let label = if option.recommended {
                format!("   ▸ {} (recommended)", option.label)
            } else {
                format!("   ▸ {}", option.label)
            };
            rows.push(Row::action(
                label,
                if option.recommended {
                    theme::focused()
                } else {
                    theme::text()
                },
                RowAction::Decide {
                    thread_id: thread.spec.id.clone(),
                    request_id: request.id.clone(),
                    choice: DecisionChoice::Option(option.id.clone()),
                },
            ));
        }
        rows.push(Row::muted("   e writes a different answer · x dismisses"));
        rows.push(Row::blank());
    }
    rows
}

/// 时间线：账本事件的用户可读投影。
fn timeline(events: &[EventResponse], pending: &[String]) -> Vec<Row> {
    let mut rows = Vec::new();
    let mut entries = Vec::new();
    for event in events {
        if let Some(entry) = Entry::project(event, pending) {
            entries.push(entry);
        }
    }
    if entries.is_empty() {
        return Vec::new();
    }
    rows.push(Row::heading(" TIMELINE"));
    for entry in entries {
        rows.push(Row::styled(
            format!(
                " {at}  {glyph} {label:<10}{text}",
                at = entry.at,
                glyph = entry.glyph,
                label = entry.label,
                text = entry.text
            ),
            entry.style,
        ));
    }
    rows.push(Row::blank());
    rows
}

struct Entry {
    at: String,
    glyph: &'static str,
    label: &'static str,
    text: String,
    style: Style,
}

impl Entry {
    fn project(event: &EventResponse, pending: &[String]) -> Option<Self> {
        let kind = event.payload.get("type").and_then(|value| value.as_str())?;
        let data = event.payload.get("data").cloned().unwrap_or_default();
        let text_of = |name: &str| -> String {
            data.get(name)
                .and_then(|value| value.as_str())
                .unwrap_or_default()
                .to_string()
        };
        let at = event
            .occurred_at
            .parse::<chrono::DateTime<chrono::Utc>>()
            .map(|time| time.format("%H:%M").to_string())
            .unwrap_or_else(|_| "     ".to_string());

        let (glyph, label, text, style) = match kind {
            "run_started" => (
                "▸",
                "started",
                format!("{} run", text_of("harness")),
                theme::text(),
            ),
            "run_finished" => {
                let success = data
                    .get("success")
                    .and_then(|value| value.as_bool())
                    .unwrap_or(false);
                if success {
                    ("✓", "finished", "run completed".to_string(), theme::done())
                } else {
                    ("✗", "finished", "run failed".to_string(), theme::critical())
                }
            }
            "harness_output" => ("·", "said", clip_lines(&text_of("text")), theme::text()),
            "model_completed" => {
                let usage = data.get("usage").cloned().unwrap_or_default();
                let input = usage
                    .get("input_tokens")
                    .and_then(|value| value.as_u64())
                    .unwrap_or(0);
                let output = usage
                    .get("output_tokens")
                    .and_then(|value| value.as_u64())
                    .unwrap_or(0);
                (
                    "·",
                    "model",
                    format!("{} · {input} in / {output} out", text_of("model")),
                    theme::muted(),
                )
            }
            "tool_called" => {
                let call = data.get("call").cloned().unwrap_or_default();
                let tool = call
                    .get("tool")
                    .and_then(|value| value.as_str())
                    .unwrap_or("tool");
                let effect = call
                    .get("effect")
                    .and_then(|value| value.as_str())
                    .unwrap_or_default();
                ("·", "tool", format!("{tool} ({effect})"), theme::text())
            }
            "opaque_tool_observed" => {
                let call = data.get("call").cloned().unwrap_or_default();
                let tool = call
                    .get("tool")
                    .and_then(|value| value.as_str())
                    .unwrap_or("tool");
                (
                    "·",
                    "tool",
                    format!("{tool} · observed, not authorized"),
                    theme::warning(),
                )
            }
            "attention_requested" => {
                let request = data.get("request").cloned().unwrap_or_default();
                let summary = request
                    .get("summary")
                    .and_then(|value| value.as_str())
                    .unwrap_or("the runtime needs you");
                let still_waiting = request
                    .get("id")
                    .and_then(|value| value.as_str())
                    .is_some_and(|id| pending.iter().any(|pending| pending == id));
                let text = if still_waiting {
                    "asked you · still waiting above".to_string()
                } else {
                    summary.to_string()
                };
                ("⚠", "needs you", text, theme::critical())
            }
            // 这两条是"这条请求被怎么处理了"。把调度结果翻译成人话，
            // 而不是把负载原样倒进主视觉。
            "attention_ruled" => {
                let ruling = data
                    .get("ruling")
                    .and_then(|ruling| ruling.get("ruling"))
                    .and_then(|value| value.as_str())
                    .unwrap_or("handled");
                let text = match ruling {
                    "interrupt" => "interrupted you right away",
                    "batch" => "held for the next batch",
                    "defer" => "deferred to a quieter moment",
                    "silent" => "handled without asking you",
                    other => other,
                };
                ("·", "attention", text.to_string(), theme::muted())
            }
            "decision_resolved" => {
                let choice = data
                    .get("decision")
                    .and_then(|decision| decision.get("choice"));
                ("·", "decided", choice_text(choice), theme::muted())
            }
            "artifact_produced" => {
                let artifact = data.get("artifact").cloned().unwrap_or_default();
                let locator = artifact
                    .get("locator")
                    .and_then(|value| value.as_str())
                    .unwrap_or("artifact");
                ("+", "artifact", locator.to_string(), theme::done())
            }
            "outcome_verified" => {
                let evidence = data
                    .get("evidence")
                    .and_then(|value| value.as_array())
                    .map(Vec::len)
                    .unwrap_or(0);
                (
                    "✓",
                    "accepted",
                    format!("{evidence} criteria backed by evidence"),
                    theme::done(),
                )
            }
            "scope_failure_recorded" => ("✗", "failed", compact(&data), theme::critical()),
            "side_effect_concluded" => ("·", "effect", text_of("note"), theme::warning()),
            "external_wait_started" => ("…", "waiting", text_of("reason"), theme::waiting()),
            "plan_proposed" | "structured_plan_proposed" => {
                let summary = data
                    .get("plan")
                    .and_then(|plan| plan.get("summary"))
                    .and_then(|value| value.as_str())
                    .map(str::to_string)
                    .unwrap_or_else(|| text_of("summary"));
                let headline = summary
                    .lines()
                    .find(|line| !line.trim().is_empty())
                    .unwrap_or_default()
                    .to_string();
                ("▸", "plan", headline, theme::text())
            }
            "memory_promoted" => ("·", "remembered", compact(&data), theme::muted()),
            _ => return None,
        };

        Some(Self {
            at,
            glyph,
            label,
            text,
            style,
        })
    }
}

fn plan(detail: &ThreadDetailData) -> Vec<Row> {
    let Some(plan) = detail.plan.current.as_ref() else {
        return Vec::new();
    };
    let mut rows = vec![Row::heading(" PLAN")];
    for step in &plan.steps {
        rows.push(Row::muted(format!(
            " {} {}",
            step_status_glyph(step.status),
            step.title
        )));
    }
    rows.push(Row::blank());
    rows
}

fn outcome(thread: &ThreadSnapshot) -> Vec<Row> {
    let mut rows = vec![Row::heading(" RESULT")];
    if thread.artifacts.is_empty() {
        rows.push(Row::muted(" nothing produced yet"));
    } else {
        for artifact in &thread.artifacts {
            // 产物要回答"在哪"，所以定位符永远在；说明只是补充。
            let note = artifact.note.trim();
            let line = if note.is_empty() || note == artifact.locator {
                artifact.locator.clone()
            } else {
                format!("{note} · {}", artifact.locator)
            };
            rows.push(Row::styled(format!(" + {line}"), theme::done()));
        }
    }
    for criterion in &thread.spec.done_when {
        rows.push(Row::muted(format!(" · {}", criterion.description)));
    }
    rows.push(Row::blank());
    rows
}

fn debug_rows(detail: &ThreadDetailData) -> Vec<Row> {
    let mut rows = vec![Row::heading(" DEBUG")];
    for scope in &detail.scopes {
        rows.push(Row::muted(format!(
            " {} {:?} generation={} failures={}",
            scope.scope_id, scope.state, scope.generation, scope.failure_count
        )));
    }
    rows.push(Row::muted(format!(
        " tokens {}/{} · tool calls {}/{} · sandbox {}s/{}s",
        detail.resources.usage.model_tokens,
        detail.resources.budget.max_model_tokens,
        detail.resources.usage.tool_calls,
        detail.resources.budget.max_tool_calls,
        detail.resources.usage.sandbox_seconds,
        detail.resources.budget.max_sandbox_seconds
    )));
    rows.push(Row::muted(format!(
        " thread {} revision {} cursor {}",
        short_id(&detail.thread.spec.id),
        detail.thread.spec.revision,
        detail.thread.cursor
    )));
    for event in &detail.events {
        rows.push(Row::muted(format!(
            " #{} {} {}",
            event.seq, event.scope_id, event.event_id
        )));
    }
    rows
}

/// agent 的输出可以是任意长；时间线里截断，剩下的留在账本里。
fn clip_lines(text: &str) -> String {
    let lines: Vec<&str> = text
        .lines()
        .filter(|line| !line.trim().is_empty())
        .collect();
    if lines.is_empty() {
        return String::new();
    }
    let mut clipped = lines
        .iter()
        .take(OUTPUT_LINES)
        .copied()
        .collect::<Vec<_>>()
        .join(" ");
    if lines.len() > OUTPUT_LINES {
        clipped.push_str(&format!(" … (+{} more lines)", lines.len() - OUTPUT_LINES));
    }
    clipped
}

/// `DecisionChoice` 是带标签的枚举：`{kind:"option",value}` /
/// `{kind:"free_text",value}` / `{kind:"dismissed"}`。
///
/// 时间线上要说人话：选了哪个、写了一句、还是干脆不回答。
fn choice_text(choice: Option<&serde_json::Value>) -> String {
    let Some(choice) = choice else {
        return "you answered".to_string();
    };
    if let Some(text) = choice.as_str() {
        return format!("you chose {text}");
    }
    let Some(object) = choice.as_object() else {
        return "you answered".to_string();
    };
    let kind = object
        .get("kind")
        .and_then(|kind| kind.as_str())
        .unwrap_or_default();
    match kind {
        "option" => object
            .get("value")
            .and_then(|value| value.as_str())
            .map(|value| format!("you chose {value}"))
            .unwrap_or_else(|| "you answered".to_string()),
        "free_text" => "you wrote an answer".to_string(),
        "dismissed" => "you dismissed it".to_string(),
        _ => "you answered".to_string(),
    }
}

/// 不知道形状的负载，压成一行短 JSON —— 宁可难看，也不要假装读懂了。
fn compact(data: &serde_json::Value) -> String {
    let text = data.to_string();
    if text.chars().count() <= 80 {
        return text;
    }
    text.chars().take(80).collect::<String>() + "…"
}

fn step_status_glyph(status: PlanStepStatus) -> &'static str {
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
