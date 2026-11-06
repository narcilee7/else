//! 屏幕与响应式渲染。视图只读 App 投影，不直接访问 Runtime。
//!
//! 整个终端界面只有三层：一行状态条、一列内容、两行页脚。
//! 没有标签页，没有卡片，没有仪表盘 —— 用户在一次会话里只做一件事：
//! 从上往下读"现在什么在等我"，然后做那个决定。

pub mod connectors;
pub mod decisions;
pub mod help;
pub mod overview;
pub mod results;
pub mod thread_detail;

use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};
use ratatui::Frame;
use unicode_width::UnicodeWidthStr;

use super::app::{App, ComposerMode, Screen};
use super::data::Data;
use super::row::Row;
use super::theme;

/// 当前屏幕的内容：一列从上往下读的行。
pub fn rows(app: &App) -> Vec<Row> {
    match (&app.screen, &app.data) {
        (Screen::Overview, Data::Overview(data)) => overview::rows(data),
        (Screen::Decisions, Data::Decisions(data)) => decisions::rows(data),
        (Screen::DecisionDetail, Data::Decisions(data)) => {
            decisions::detail_rows(data, app.focused_decision.as_ref())
        }
        (Screen::Results, Data::Threads(threads)) => results::rows(threads),
        (Screen::ThreadDetail, Data::ThreadDetail(detail)) => {
            thread_detail::rows(detail, app.debug_detail)
        }
        (Screen::Connectors, Data::Threads(threads)) => results::thread_picker_rows(threads),
        (Screen::Connectors, Data::Connectors(rows)) => {
            connectors::rows(rows, app.focused_thread.as_ref())
        }
        (Screen::Composer, _) => Vec::new(),
        _ => loading_rows(app),
    }
}

/// 数据还没到位时，界面只说自己正在干什么，不假装自己是空的。
fn loading_rows(app: &App) -> Vec<Row> {
    if app.status.as_ref().is_some_and(|status| status.failed) {
        vec![
            Row::styled("This view could not be loaded", theme::critical()),
            Row::muted("Press r to retry. The previous data has not been replaced."),
        ]
    } else {
        vec![Row::muted("Loading the latest state…")]
    }
}

pub fn flat_rows(app: &App) -> Vec<Row> {
    rows(app)
}

pub fn thread_state_label(state: else_protocol::thread::ThreadState) -> &'static str {
    use else_protocol::thread::ThreadState;
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

pub fn thread_style(state: else_protocol::thread::ThreadState) -> Style {
    use else_protocol::thread::ThreadState;
    match state {
        ThreadState::Completed | ThreadState::Cancelled => theme::done(),
        ThreadState::NeedsDecision | ThreadState::Failed => theme::critical(),
        ThreadState::WaitingExternal => theme::waiting(),
        ThreadState::Paused => theme::muted(),
        ThreadState::Active => theme::text(),
    }
}

pub fn short_id(id: &else_protocol::ids::ThreadId) -> String {
    id.to_string().chars().take(8).collect()
}

/// "这个决定有多贵"：界面上所有地方用同一套说法和颜色。
pub fn loss_label(loss: else_protocol::attention::ExpectedLoss) -> &'static str {
    use else_protocol::attention::ExpectedLoss;
    match loss {
        ExpectedLoss::Low => "low",
        ExpectedLoss::Medium => "medium",
        ExpectedLoss::High => "high",
        ExpectedLoss::Critical => "critical",
    }
}

pub fn loss_style(loss: else_protocol::attention::ExpectedLoss) -> Style {
    use else_protocol::attention::ExpectedLoss;
    match loss {
        ExpectedLoss::Low => theme::text(),
        ExpectedLoss::Medium => theme::waiting(),
        ExpectedLoss::High | ExpectedLoss::Critical => theme::critical(),
    }
}

pub fn render(frame: &mut Frame<'_>, app: &App) {
    let areas = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(2),
        ])
        .split(frame.area());

    draw_status_line(frame, areas[0], app);
    if app.screen == Screen::Composer {
        draw_composer(frame, areas[1], app);
        draw_footer(frame, areas[2], app, &[]);
    } else {
        // 一帧只投影一次：内容区和页脚必须看同一份行。
        let rows = rows(app);
        draw_content(frame, areas[1], app.cursor, &rows);
        draw_footer(frame, areas[2], app, &rows);
    }
    if app.show_help {
        draw_help(frame, frame.area());
    }
    if app.confirmation.is_some() {
        draw_confirmation(frame, frame.area());
    }
}

/// 第一行：左边"我在哪"，右边"现在是什么状况"。
///
/// 这一行永远只有一行。它同时承担了旧版藏在 `doctor` 里的诚实义务：
/// 常驻还是临时 Runtime、有多少事在等人、同步是否在进行。
fn draw_status_line(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let left = Line::from(vec![
        Span::styled("else", theme::heading()),
        Span::styled(format!(" · {}", crumb(app)), theme::muted()),
    ]);
    let right = Line::from(join_segments(status_segments(app)));
    let right_width = right.width().min(area.width as usize) as u16;
    frame.render_widget(
        Paragraph::new(left),
        Rect {
            width: area.width.saturating_sub(right_width),
            ..area
        },
    );
    frame.render_widget(
        Paragraph::new(right).alignment(Alignment::Right),
        Rect {
            x: area.x + area.width.saturating_sub(right_width),
            width: right_width,
            ..area
        },
    );
}

/// 你正在看的是哪一件东西。屏幕不是功能模块，是任务的一个子集。
fn crumb(app: &App) -> String {
    match app.screen {
        Screen::Overview => "now".into(),
        // 目标本身是正文的标题；这里只回答"我在哪一个"。
        Screen::ThreadDetail => match &app.data {
            Data::ThreadDetail(detail) => format!("thread {}", short_id(&detail.thread.spec.id)),
            _ => "thread".into(),
        },
        Screen::Decisions => "decisions".into(),
        Screen::DecisionDetail => "decisions · review".into(),
        Screen::Results => "results".into(),
        Screen::Connectors => "connections".into(),
        Screen::Composer => "new thread".into(),
    }
}

/// 状态条右半：只有真的有话要说的段才出现。
fn status_segments(app: &App) -> Vec<Span<'static>> {
    let mut segments = Vec::new();
    if app.screen == Screen::ThreadDetail {
        if let Data::ThreadDetail(detail) = &app.data {
            let thread = &detail.thread;
            segments.push(Span::styled(
                thread_state_label(thread.state),
                thread_style(thread.state),
            ));
            // 花掉多少是判断"还要不要继续"的依据；零就别念出来。
            if thread.resource_usage.model_tokens > 0 {
                segments.push(Span::styled(
                    format!("{} tokens", thread.resource_usage.model_tokens),
                    theme::muted(),
                ));
            }
            if thread.resource_usage.model_spend_usd_micros > 0 {
                let micros = thread.resource_usage.model_spend_usd_micros;
                // 花了几分钱就说几分钱，四舍五入成 $0.00 等于没报。
                let usd = micros as f64 / 1_000_000.0;
                let text = if micros < 10_000 {
                    format!("${usd:.4}")
                } else {
                    format!("${usd:.2}")
                };
                segments.push(Span::styled(text, theme::muted()));
            }
        }
    } else {
        let (waiting, running) = counts(app);
        if waiting > 0 {
            segments.push(Span::styled(
                format!("{waiting} waiting"),
                theme::critical(),
            ));
        }
        if running > 0 {
            segments.push(Span::styled(format!("{running} running"), theme::text()));
        }
    }
    if app.is_loading() {
        segments.push(Span::styled("syncing", theme::muted()));
    }
    // 用的是常驻 Runtime 还是命令自己起的临时替身 —— 写在最显眼的地方。
    if let Some(note) = app.runtime_note.as_deref().filter(|note| !note.is_empty()) {
        let _ = note;
        segments.push(Span::styled("runtime temp", theme::warning()));
    }
    segments
}

fn join_segments(segments: Vec<Span<'static>>) -> Vec<Span<'static>> {
    let mut line = Vec::new();
    for (index, segment) in segments.into_iter().enumerate() {
        if index > 0 {
            line.push(Span::styled(" · ", theme::muted()));
        }
        line.push(segment);
    }
    line
}

/// 首屏两个数字：多少事在等人、多少事在跑。
fn counts(app: &App) -> (usize, usize) {
    let Data::Overview(data) = &app.data else {
        return (0, 0);
    };
    (
        waiting_count(data),
        data.threads
            .iter()
            .filter(|thread| {
                matches!(
                    thread.state,
                    else_protocol::thread::ThreadState::Active
                        | else_protocol::thread::ThreadState::WaitingExternal
                )
            })
            .count(),
    )
}

/// 有多少件事正在等人裁决。窗口标题和首屏用的是同一个数。
pub fn waiting_count(data: &super::data::OverviewData) -> usize {
    data.threads
        .iter()
        .map(|thread| thread.decisions.len())
        .sum::<usize>()
        + data
            .attention
            .iter()
            .map(|window| window.requests.len())
            .sum::<usize>()
}

/// 内容区：一列行，没有边框。
///
/// 画边要吃掉 2 行 2 列，换来的只是一个标题；而分组标题本来就能自己说话。
fn draw_content(frame: &mut Frame<'_>, area: Rect, cursor: usize, rows: &[Row]) {
    if area.width == 0 || area.height == 0 || rows.is_empty() {
        return;
    }

    let heights: Vec<u16> = rows.iter().map(|row| row_height(row, area.width)).collect();
    let mut start = 0usize;
    while start < cursor
        && heights[start..=cursor.min(rows.len() - 1)]
            .iter()
            .map(|height| *height as usize)
            .sum::<usize>()
            > area.height as usize
    {
        start += 1;
    }

    let mut y = area.y;
    for (index, row) in rows.iter().enumerate().skip(start) {
        let height = heights[index];
        if y.saturating_add(height) > area.y + area.height {
            break;
        }
        draw_row(
            frame,
            Rect {
                x: area.x,
                y,
                width: area.width,
                height,
            },
            row,
            index == cursor,
        );
        y = y.saturating_add(height);
    }
}

fn row_height(row: &Row, width: u16) -> u16 {
    // 带右对齐列的行不折行：折了就不再"齐"，也就没法竖着比。
    if row.meta.is_some() {
        return 1;
    }
    let lines = row
        .text
        .lines()
        .map(|line| {
            UnicodeWidthStr::width(line)
                .max(1)
                .div_ceil(width as usize)
                .max(1)
        })
        .sum::<usize>();
    lines.max(1) as u16
}

fn draw_row(frame: &mut Frame<'_>, area: Rect, row: &Row, selected: bool) {
    let width = area.width as usize;
    if let Some(meta) = row.meta.as_deref() {
        let meta_width = UnicodeWidthStr::width(meta);
        let text = truncate(&row.text, width.saturating_sub(meta_width + 2));
        let gap = width.saturating_sub(UnicodeWidthStr::width(text.as_str()) + meta_width);
        let paragraph = if selected {
            Paragraph::new(Span::styled(
                format!("{text}{}{meta}", " ".repeat(gap)),
                theme::selected(),
            ))
        } else {
            Paragraph::new(Line::from(vec![
                Span::styled(text, row.style),
                Span::raw(" ".repeat(gap)),
                Span::styled(meta.to_string(), theme::muted()),
            ]))
        };
        frame.render_widget(paragraph, area);
        return;
    }

    let paragraph = Paragraph::new(row.text.clone()).wrap(Wrap { trim: false });
    let paragraph = if selected {
        paragraph.style(theme::selected())
    } else {
        paragraph.style(row.style)
    };
    frame.render_widget(paragraph, area);
}

/// 按显示宽度截断，不切断宽字符；被截断时留一个省略号，别假装那就是全文。
fn truncate(text: &str, limit: usize) -> String {
    if UnicodeWidthStr::width(text) <= limit {
        return text.to_string();
    }
    let mut out = String::new();
    let mut used = 0usize;
    for character in text.chars() {
        let width = UnicodeWidthStr::width(character.to_string().as_str());
        if used + width + 1 > limit {
            break;
        }
        used += width;
        out.push(character);
    }
    out.push('…');
    out
}

/// 页脚：第一行是"现在能按什么"，第二行是"刚发生了什么 / 这一行是什么"。
///
/// 键提示按当前屏幕与当前选择变化，不是一张永远贴在底下的说明书。
fn draw_footer(frame: &mut Frame<'_>, area: Rect, app: &App, rows: &[Row]) {
    let hints = hints(app);
    let mut line = Vec::new();
    for (index, (key, action)) in hints.iter().enumerate() {
        if index > 0 {
            line.push(Span::styled("  ", theme::hint()));
        }
        line.push(Span::styled(*key, theme::hint_key()));
        line.push(Span::styled(format!(" {action}"), theme::hint()));
    }

    let context = footer_context(app, rows);
    let mut lines = vec![Line::from(line)];
    if let Some(context) = context {
        lines.push(context);
    }
    frame.render_widget(Paragraph::new(lines), area);
}

fn footer_context(app: &App, rows: &[Row]) -> Option<Line<'static>> {
    if let Some(status) = app.status.as_ref() {
        return Some(Line::from(Span::styled(
            status.text.clone(),
            if status.failed {
                theme::critical()
            } else {
                theme::done()
            },
        )));
    }
    let note = rows.get(app.cursor).and_then(|row| row.note.clone())?;
    Some(Line::from(Span::styled(note, theme::muted())))
}

fn hints(app: &App) -> Vec<(&'static str, &'static str)> {
    let mut hints: Vec<(&'static str, &'static str)> = match app.screen {
        Screen::Overview => vec![("⏎", "open"), ("d", "decisions"), ("n", "new")],
        Screen::Decisions => vec![("⏎", "review"), ("n", "new"), ("Esc", "back")],
        Screen::DecisionDetail => vec![("⏎", "choose"), ("e", "answer"), ("x", "dismiss")],
        Screen::Results => vec![("⏎", "inspect"), ("Esc", "back")],
        Screen::ThreadDetail => vec![
            ("⏎", "open"),
            ("a", "advance"),
            ("e", "answer"),
            ("d", "debug"),
            ("Esc", "back"),
        ],
        Screen::Connectors => vec![("⏎", "sync"), ("Esc", "back")],
        Screen::Composer => vec![("⏎", "create"), ("Esc", "cancel")],
    };
    // 页脚是给此刻用的，不是索引。剩下的进 `?`。
    hints.truncate(5);
    hints.push(("?", "keys"));
    hints
}

fn draw_composer(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let prompt = match app.composer_mode {
        ComposerMode::Thread => "What should else take care of?",
        ComposerMode::Decision { .. } => "How should the runtime proceed?",
    };
    if area.height < 4 {
        return;
    }
    frame.render_widget(
        Paragraph::new(prompt).style(theme::muted()),
        Rect { height: 1, ..area },
    );

    let chars: Vec<char> = app.composer.text().chars().collect();
    let cursor = app.composer.cursor().min(chars.len());
    let before: String = chars[..cursor].iter().collect();
    let current = chars.get(cursor).copied().unwrap_or(' ');
    let after: String = if cursor < chars.len() {
        chars[cursor + 1..].iter().collect()
    } else {
        String::new()
    };
    let input = Line::from(vec![
        Span::styled("> ", theme::muted()),
        Span::styled(before, theme::text()),
        Span::styled(current.to_string(), theme::selected()),
        Span::styled(after, theme::text()),
    ]);
    frame.render_widget(
        Paragraph::new(input).wrap(Wrap { trim: false }),
        Rect {
            y: area.y + 2,
            height: area.height.saturating_sub(2),
            ..area
        },
    );
}

fn draw_help(frame: &mut Frame<'_>, area: Rect) {
    let popup = centered_rect(area, 72, 22);
    frame.render_widget(Clear, popup);
    frame.render_widget(
        Paragraph::new(help::lines())
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(Span::styled(" keys ", theme::heading())),
            )
            .wrap(Wrap { trim: false }),
        popup,
    );
}

fn draw_confirmation(frame: &mut Frame<'_>, area: Rect) {
    let popup = centered_rect(area, 64, 9);
    frame.render_widget(Clear, popup);
    let lines = vec![
        Line::from("This approval has a high-impact external effect."),
        Line::from(""),
        Line::from("Review the command and its consequences before continuing."),
        Line::from(""),
        Line::from(vec![
            Span::styled("Enter", theme::hint_key()),
            Span::styled(" confirm approval   ", theme::text()),
            Span::styled("Esc", theme::hint_key()),
            Span::styled(" cancel", theme::text()),
        ]),
    ];
    frame.render_widget(
        Paragraph::new(lines)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(Span::styled(" Confirm approval ", theme::critical())),
            )
            .wrap(Wrap { trim: false }),
        popup,
    );
}

fn centered_rect(area: Rect, max_width: u16, max_height: u16) -> Rect {
    let width = area.width.saturating_sub(4).min(max_width);
    let height = area.height.saturating_sub(2).min(max_height);
    Rect {
        x: area.x + area.width.saturating_sub(width) / 2,
        y: area.y + area.height.saturating_sub(height) / 2,
        width,
        height,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::data::{OverviewData, ThreadDetailData};
    use else_protocol::api::{EventResponse, PlanView, ResourceView, ThreadSnapshot};
    use else_protocol::attention::{AttentionKind, AttentionRequest, DecisionOption, ExpectedLoss};
    use else_protocol::ids::{AttentionId, ThreadId};
    use else_protocol::thread::{AttentionPolicy, Authority, ThreadMode, ThreadSpec, ThreadState};
    use else_protocol::world::PersonalBrief;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;
    use serde_json::json;

    fn render_text(app: &App, width: u16, height: u16) -> String {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| render(frame, app)).unwrap();
        let buffer = terminal.backend().buffer();
        let mut text = String::new();
        for y in 0..height {
            for x in 0..width {
                text.push_str(buffer[(x, y)].symbol());
            }
            text.push('\n');
        }
        text
    }

    fn app() -> App {
        let (loads, _load_rx) = std::sync::mpsc::channel();
        let (actions, _action_rx) = std::sync::mpsc::channel();
        App::new(loads, actions)
    }

    fn request(id: &str, summary: &str) -> AttentionRequest {
        AttentionRequest {
            id: AttentionId::from(id),
            kind: AttentionKind::Authorization,
            summary: summary.into(),
            blocked_reason: "it changes a system outside the workspace".into(),
            expected_loss: ExpectedLoss::High,
            blocking: true,
            reversible: false,
            decide_by: None,
            options: vec![DecisionOption {
                id: "approve".into(),
                label: "Allow once".into(),
                recommended: true,
            }],
            default_option: Some("approve".into()),
            related_call_key: None,
        }
    }

    fn thread(
        id: &str,
        outcome: &str,
        state: ThreadState,
        decisions: Vec<AttentionRequest>,
    ) -> ThreadSnapshot {
        ThreadSnapshot {
            spec: ThreadSpec {
                id: ThreadId::from(id),
                revision: 1,
                outcome: outcome.into(),
                done_when: Vec::new(),
                constraints: Vec::new(),
                authority: Authority {
                    max_autonomous_effect: else_protocol::effect::SideEffectKind::ReadOnly,
                    forbidden: Vec::new(),
                },
                attention_policy: AttentionPolicy {
                    allow_interrupt: true,
                    batch_interval_minutes: 15,
                },
                mode: ThreadMode::default(),
                opaque_harness_policy: Default::default(),
                deadline: None,
                assumptions: Vec::new(),
                project: None,
                origin: None,
                connector_policy: Default::default(),
            },
            state,
            decisions,
            artifacts: Vec::new(),
            resource_budget: Default::default(),
            resource_usage: Default::default(),
            cursor: 0,
            operation_id: None,
        }
    }

    fn overview(threads: Vec<ThreadSnapshot>) -> Data {
        Data::Overview(Box::new(OverviewData {
            brief: PersonalBrief {
                generated_at: chrono::Utc::now(),
                open_commitments: Vec::new(),
                waiting_external: Vec::new(),
                overdue: Vec::new(),
                action_items_due_soon: Vec::new(),
                risks: Vec::new(),
                upcoming_calendar: Vec::new(),
            },
            attention: Vec::new(),
            threads,
        }))
    }

    fn event(seq: u64, at: &str, payload: serde_json::Value) -> EventResponse {
        EventResponse {
            seq,
            event_id: format!("e-{seq}"),
            thread_id: "thread-1".into(),
            scope_id: "scope-1".into(),
            run_id: None,
            generation: 1,
            thread_revision: 1,
            occurred_at: at.into(),
            payload,
        }
    }

    fn thread_detail(snapshot: ThreadSnapshot, events: Vec<EventResponse>) -> Data {
        Data::ThreadDetail(Box::new(ThreadDetailData {
            thread: snapshot,
            scopes: Vec::new(),
            plan: PlanView {
                current: None,
                history: Vec::new(),
                impacts: Vec::new(),
            },
            resources: ResourceView {
                budget: Default::default(),
                usage: Default::default(),
                sandboxes: Vec::new(),
            },
            events,
        }))
    }

    #[test]
    fn now_puts_what_waits_for_you_above_everything_else() {
        let mut app = app();
        app.data = overview(vec![
            thread(
                "thread-1",
                "Prepare the launch deck",
                ThreadState::NeedsDecision,
                vec![request("request-1", "Approve running a shell command")],
            ),
            thread(
                "thread-2",
                "Rewrite the pricing page",
                ThreadState::Active,
                Vec::new(),
            ),
        ]);
        let content = render_text(&app, 110, 30);
        let needs_you = content.find("NEEDS YOU").expect("needs-you section");
        let running = content.find("RUNNING").expect("running section");
        assert!(
            needs_you < running,
            "the thing that waits for you must come first:\n{content}"
        );
        assert!(content.contains("Approve running a shell command"));
    }

    #[test]
    fn now_caps_each_section_and_says_where_the_rest_lives() {
        let threads = (0..5)
            .map(|index| {
                thread(
                    &format!("thread-{index}"),
                    &format!("Task {index}"),
                    ThreadState::NeedsDecision,
                    vec![request(&format!("request-{index}"), "Approve a step")],
                )
            })
            .collect();
        let mut app = app();
        app.data = overview(threads);
        let content = render_text(&app, 110, 30);
        assert_eq!(
            content.matches('▸').count(),
            3,
            "at most three rows:\n{content}"
        );
        assert!(content.contains("+2 more · d reviews all of them"));
    }

    #[test]
    fn now_empty_state_teaches_the_next_action() {
        let mut app = app();
        app.data = overview(Vec::new());
        let content = render_text(&app, 100, 24);
        assert!(content.contains("Nothing is running yet."));
        assert!(content.contains("Press n and describe an outcome"));
    }

    /// 负载形状全部照抄一次真实脚本化运行的账本输出，
    /// 而不是照着枚举猜出来的。
    #[test]
    fn thread_screen_projects_ledger_events_into_a_readable_timeline() {
        let snapshot = thread(
            "thread-1",
            "Prepare the launch deck",
            ThreadState::NeedsDecision,
            Vec::new(),
        );
        let mut app = app();
        app.screen = Screen::ThreadDetail;
        app.focused_thread = Some(ThreadId::from("thread-1"));
        app.data = thread_detail(
            snapshot,
            vec![
                event(
                    1,
                    "2026-11-06T17:09:15.552462+00:00",
                    json!({"type": "run_started", "data": {"run_id": "f0c23022", "harness": "scripted", "capabilities": null}}),
                ),
                // 账本里有、但界面上不该出现的运维事件。
                event(
                    2,
                    "2026-11-06T17:09:15.553000+00:00",
                    json!({"type": "context_compiled", "data": {"snapshot": {"id": "ad0d8221"}}}),
                ),
                event(
                    3,
                    "2026-11-06T17:09:15.554000+00:00",
                    json!({"type": "model_completed", "data": {"model": "mock-model", "usage": {"input_tokens": 21, "output_tokens": 64, "cost_usd_micros": 1500, "basis": "estimated"}}}),
                ),
                event(
                    4,
                    "2026-11-06T17:09:15.554462+00:00",
                    json!({"type": "tool_called", "data": {"call": {"id": "c193113a", "tool": "fs.write", "effect": "reversible", "input": {"path": "plan.md"}, "idempotency_key": "plan-write"}}}),
                ),
                event(
                    5,
                    "2026-11-06T17:09:15.555823+00:00",
                    json!({"type": "artifact_produced", "data": {"artifact": {"id": "cc93ebb6", "kind": "document", "locator": "plan.md", "manifest": null, "produced_by": "f0c23022", "note": "执行计划"}}}),
                ),
                event(
                    6,
                    "2026-11-06T17:09:15.558864+00:00",
                    json!({"type": "run_finished", "data": {"run_id": "f0c23022", "success": true}}),
                ),
            ],
        );
        let content = render_text(&app, 120, 34);
        assert!(content.contains("TIMELINE"), "timeline section:\n{content}");
        assert!(
            content.contains("17:09"),
            "events carry their time:\n{content}"
        );
        assert!(content.contains("scripted run"));
        assert!(content.contains("21 in / 64 out"));
        assert!(content.contains("fs.write (reversible)"));
        assert!(content.contains("+ artifact"));
        assert!(content.contains("plan.md"));
        assert!(content.contains("run completed"));
        // 事件 id 和内部事件类型是调试信息，不该出现在主视觉里。
        assert!(!content.contains("e-1"));
        assert!(!content.contains("context_compiled"));
    }

    #[test]
    fn thread_screen_offers_the_pending_decision_inline() {
        let snapshot = thread(
            "thread-1",
            "Prepare the launch deck",
            ThreadState::NeedsDecision,
            vec![request("request-1", "Overwrite outline.md?")],
        );
        let mut app = app();
        app.screen = Screen::ThreadDetail;
        app.focused_thread = Some(ThreadId::from("thread-1"));
        app.data = thread_detail(snapshot, Vec::new());
        let rows = super::rows(&app);
        let option = rows
            .iter()
            .find(|row| row.text.contains("Allow once"))
            .expect("the option is a row on the timeline");
        assert!(
            matches!(
                option.action,
                Some(crate::tui::row::RowAction::Decide { .. })
            ),
            "the option must be directly actionable"
        );
        let content = render_text(&app, 110, 26);
        assert!(content.contains("needs you"));
        assert!(content.contains("it changes a system outside the workspace"));
    }

    #[test]
    fn status_line_says_when_the_runtime_is_only_a_stand_in() {
        let mut app = app();
        app.runtime_note = Some("runtime temp".into());
        let content = render_text(&app, 100, 20);
        assert!(
            content.contains("runtime temp"),
            "honesty is not hidden:\n{content}"
        );
    }

    #[test]
    fn every_screen_renders_at_awkward_sizes() {
        let mut app = app();
        app.data = overview(vec![thread(
            "thread-1",
            "A rather long outcome that will not fit in a narrow terminal without being clipped",
            ThreadState::NeedsDecision,
            vec![request("request-1", "Approve running a shell command")],
        )]);
        for (width, height) in [(120, 40), (80, 24), (60, 18), (40, 12)] {
            let content = render_text(&app, width, height);
            assert!(
                content.contains("else"),
                "{width}x{height}:
{content}"
            );
        }

        app.screen = Screen::ThreadDetail;
        app.data = thread_detail(
            thread(
                "thread-1",
                "Prepare the launch deck",
                ThreadState::Active,
                Vec::new(),
            ),
            vec![event(
                1,
                "2026-10-31T09:31:00Z",
                json!({"type": "run_started", "data": {"run_id": "run-1", "harness": "claude-code"}}),
            )],
        );
        for (width, height) in [(120, 40), (80, 24), (60, 18), (40, 12)] {
            let content = render_text(&app, width, height);
            assert!(
                content.contains("TIMELINE"),
                "{width}x{height}:
{content}"
            );
        }
    }

    #[test]
    fn footer_offers_the_keys_that_work_here() {
        let app = app();
        let content = render_text(&app, 100, 20);
        assert!(content.contains("decisions"), "d is reachable:\n{content}");
        assert!(
            !content.contains("Today"),
            "no tab bar survives:\n{content}"
        );
        assert!(content.contains("keys"), "? is always offered:\n{content}");
    }
}
