//! 屏幕与响应式渲染。视图只读 App 投影，不直接访问 Runtime。

pub mod connectors;
pub mod decisions;
pub mod help;
pub mod overview;
pub mod results;
pub mod thread_detail;

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};
use ratatui::Frame;

use super::app::{App, ComposerMode, Screen};
use super::data::Data;
use super::row::{Pane, Row};
use super::theme;

pub fn panes(app: &App) -> Vec<Pane> {
    match (&app.screen, &app.data) {
        (Screen::Overview, Data::Overview(data)) => {
            vec![overview::attention_pane(data), overview::world_pane(data)]
        }
        (Screen::Decisions, Data::Decisions(data)) => vec![decisions::pane(data)],
        (Screen::DecisionDetail, Data::Decisions(data)) => {
            vec![decisions::detail_pane(data, app.focused_decision.as_ref())]
        }
        (Screen::Results, Data::Threads(threads)) => vec![results::pane(threads)],
        (Screen::ThreadDetail, Data::ThreadDetail(detail)) => {
            vec![thread_detail::pane(detail, app.debug_detail)]
        }
        (Screen::ThreadDetail, Data::Threads(threads))
        | (Screen::Connectors, Data::Threads(threads)) => vec![results::thread_picker(threads)],
        (Screen::Connectors, Data::Connectors(rows)) => {
            vec![connectors::pane(rows, app.focused_thread.as_ref())]
        }
        _ => vec![Pane {
            title: app.screen.title().to_string(),
            rows: if app.status.as_ref().is_some_and(|status| status.failed) {
                vec![
                    Row::styled("Could not load this view", theme::critical()),
                    Row::muted("Press r to retry. Your previous data has not been replaced."),
                ]
            } else {
                vec![Row::muted("Loading the latest state…")]
            },
        }],
    }
}

pub fn flat_rows(app: &App) -> Vec<Row> {
    panes(app).into_iter().flat_map(|pane| pane.rows).collect()
}

pub fn thread_state_label(state: else_protocol::thread::ThreadState) -> &'static str {
    use else_protocol::thread::ThreadState;
    match state {
        ThreadState::Active => "Active",
        ThreadState::WaitingExternal => "Waiting",
        ThreadState::NeedsDecision => "Needs you",
        ThreadState::Paused => "Paused",
        ThreadState::Completed => "Completed",
        ThreadState::Cancelled => "Cancelled",
        ThreadState::Failed => "Failed",
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

pub fn render(frame: &mut Frame<'_>, app: &App) {
    let areas = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(1),
            Constraint::Length(2),
        ])
        .split(frame.area());

    draw_header(frame, areas[0], app);
    if app.screen == Screen::Composer {
        draw_composer(frame, areas[1], app);
    } else {
        draw_body(frame, areas[1], app);
    }
    draw_footer(frame, areas[2], app);
    if app.show_help {
        draw_help(frame, frame.area());
    }
    if app.confirmation.is_some() {
        draw_confirmation(frame, frame.area());
    }
}

fn draw_header(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let mut tabs = Vec::new();
    for (index, (screen, label)) in [
        (Screen::Overview, "1 Today"),
        (Screen::Decisions, "2 Decisions"),
        (Screen::Results, "3 Results"),
        (Screen::Connectors, "4 Settings"),
    ]
    .iter()
    .enumerate()
    {
        if index > 0 {
            tabs.push(Span::styled("  ", theme::muted()));
        }
        let active = *screen == app.screen
            || (*screen == Screen::Decisions && app.screen == Screen::DecisionDetail);
        tabs.push(Span::styled(
            *label,
            if active {
                theme::focused()
            } else {
                theme::hint()
            },
        ));
    }
    if app.is_loading() {
        tabs.push(Span::styled("  · syncing…", theme::muted()));
    }
    let title = if area.width < 70 {
        Line::from(Span::styled("else", theme::heading()))
    } else {
        Line::from(vec![
            Span::styled("else ", theme::heading()),
            Span::styled("you do what matters", theme::muted()),
        ])
    };
    frame.render_widget(
        Paragraph::new(Line::from(tabs)).block(Block::default().borders(Borders::ALL).title(title)),
        area,
    );
}

fn draw_body(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let panes = panes(app);
    if panes.len() >= 2 {
        let direction = if area.width >= 100 {
            Direction::Horizontal
        } else {
            Direction::Vertical
        };
        let regions = Layout::default()
            .direction(direction)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(area);
        let mut offset = 0usize;
        for (index, pane) in panes.iter().take(2).enumerate() {
            draw_pane(frame, regions[index], pane, offset, app.cursor);
            offset += pane.rows.len();
        }
        return;
    }
    if let Some(pane) = panes.first() {
        draw_pane(frame, area, pane, 0, app.cursor);
    }
}

fn draw_pane(frame: &mut Frame<'_>, area: Rect, pane: &Pane, offset: usize, cursor: usize) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(Span::styled(format!(" {} ", pane.title), theme::heading()));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let paragraph_for = |row: &Row, selected: bool| {
        Paragraph::new(row.text.clone())
            .style(if selected {
                theme::selected()
            } else {
                row.style
            })
            .wrap(Wrap { trim: false })
    };
    let heights: Vec<u16> = pane
        .rows
        .iter()
        .map(|row| {
            row.text
                .lines()
                .map(|line| {
                    let width = Line::from(line).width();
                    width.max(1).div_ceil(inner.width as usize).max(1)
                })
                .sum::<usize>()
                .max(1) as u16
        })
        .collect();
    let selected = (cursor >= offset && cursor < offset + pane.rows.len()).then(|| cursor - offset);
    let mut start = 0usize;
    if let Some(selected) = selected {
        while start < selected
            && heights[start..=selected]
                .iter()
                .map(|height| *height as usize)
                .sum::<usize>()
                > inner.height as usize
        {
            start += 1;
        }
    }

    let mut y = inner.y;
    for (index, row) in pane.rows.iter().enumerate().skip(start) {
        let height = heights[index];
        if y.saturating_add(height) > inner.y + inner.height {
            break;
        }
        let selected = offset + index == cursor;
        frame.render_widget(
            paragraph_for(row, selected),
            Rect {
                x: inner.x,
                y,
                width: inner.width,
                height,
            },
        );
        y = y.saturating_add(height);
    }
}

fn draw_composer(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let (title, prompt) = match app.composer_mode {
        ComposerMode::Thread => ("New thread", "What outcome should Else own?"),
        ComposerMode::Decision { .. } => ("Custom response", "How should the runtime proceed?"),
    };
    let outer = Block::default()
        .borders(Borders::ALL)
        .title(Span::styled(format!(" {title} "), theme::heading()));
    let inner = outer.inner(area);
    frame.render_widget(outer, area);
    if inner.height < 3 {
        return;
    }
    frame.render_widget(
        Paragraph::new(prompt).style(theme::muted()),
        Rect { height: 1, ..inner },
    );

    let input_area = Rect {
        x: inner.x,
        y: inner.y + 2,
        width: inner.width,
        height: 3.min(inner.height.saturating_sub(2)),
    };
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
        Span::styled(before, theme::text()),
        Span::styled(current.to_string(), theme::selected()),
        Span::styled(after, theme::text()),
    ]);
    frame.render_widget(
        Paragraph::new(input)
            .block(Block::default().borders(Borders::ALL).title("Your answer"))
            .wrap(Wrap { trim: false }),
        input_area,
    );
    let hint = Paragraph::new(Line::from(vec![
        Span::styled("Enter", theme::hint_key()),
        Span::styled(" submit  ", theme::hint()),
        Span::styled("Esc", theme::hint_key()),
        Span::styled(" cancel  ", theme::hint()),
        Span::styled("paste", theme::hint_key()),
        Span::styled(" supported", theme::hint()),
    ]));
    frame.render_widget(
        hint,
        Rect {
            x: inner.x,
            y: inner.y + inner.height.saturating_sub(1),
            width: inner.width,
            height: 1,
        },
    );
}

fn draw_footer(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let hint = match app.screen {
        Screen::Overview => {
            "Enter open/complete · n new · r refresh · 1-4 screens · ? help · q quit"
        }
        Screen::Decisions => "Enter review · n new · r refresh · Esc back · ? help · q quit",
        Screen::Results => "Enter inspect outcome · n new · r refresh · ? help · q quit",
        Screen::ThreadDetail => "a advance · d debug · r refresh · Esc back · ? help · q quit",
        Screen::DecisionDetail => "Enter choose · e respond · x dismiss · Esc back · q quit",
        Screen::Connectors => "Enter/s sync · r refresh · Esc back · ? help · q quit",
        Screen::Composer => "Enter submit · Esc cancel · Ctrl-C quit",
    };
    let status = app.status.as_ref().map(|status| {
        Span::styled(
            status.text.clone(),
            if status.failed {
                theme::critical()
            } else {
                theme::done()
            },
        )
    });
    let lines = vec![
        Line::from(Span::styled(hint, theme::hint())),
        status.map_or_else(|| Line::from(""), |span| Line::from(vec![span])),
    ];
    frame.render_widget(Paragraph::new(lines), area);
}

fn draw_help(frame: &mut Frame<'_>, area: Rect) {
    let popup = centered_rect(area, 76, 26);
    frame.render_widget(Clear, popup);
    frame.render_widget(
        Paragraph::new(help::lines())
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(Span::styled(" Keys ", theme::heading())),
            )
            .wrap(Wrap { trim: false }),
        popup,
    );
}

fn draw_confirmation(frame: &mut Frame<'_>, area: Rect) {
    let popup = centered_rect(area, 64, 9);
    frame.render_widget(Clear, popup);
    let lines = vec![
        Line::from("This approval may have a high-impact external effect."),
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
    use crate::tui::data::DecisionsData;
    use else_protocol::attention::{AttentionKind, AttentionRequest, DecisionOption, ExpectedLoss};
    use else_protocol::ids::{AttentionId, ThreadId};
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn render_text(app: &App, width: u16, height: u16) -> String {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| render(frame, app)).unwrap();
        terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect()
    }

    fn app() -> App {
        let (loads, _load_rx) = std::sync::mpsc::channel();
        let (actions, _action_rx) = std::sync::mpsc::channel();
        App::new(loads, actions)
    }

    #[test]
    fn initial_view_renders_at_standard_and_narrow_sizes() {
        for (width, height) in [(120, 32), (80, 24), (60, 20)] {
            let content = render_text(&app(), width, height);
            assert!(content.contains("Today"));
            assert!(content.contains("Decisions"));
            assert!(content.contains("Results"));
        }
    }

    #[test]
    fn results_empty_state_explains_what_will_appear() {
        let mut app = app();
        app.screen = Screen::Results;
        app.data = Data::Threads(Vec::new());
        let content = render_text(&app, 100, 24);
        assert!(content.contains("No finished threads yet."));
        assert!(content.contains("Completed outcomes"));
    }

    #[test]
    fn decision_detail_renders_context_and_safe_actions() {
        let mut app = app();
        app.screen = Screen::DecisionDetail;
        app.focused_decision = Some(crate::tui::app::DecisionFocus {
            thread_id: ThreadId::from("thread-1"),
            request_id: AttentionId::from("request-1"),
        });
        app.data = Data::Decisions(Box::new(DecisionsData {
            windows: Vec::new(),
            loose: vec![(
                ThreadId::from("thread-1"),
                AttentionRequest {
                    id: AttentionId::from("request-1"),
                    kind: AttentionKind::Authorization,
                    summary: "Publish the release?".into(),
                    blocked_reason: "This changes the production service".into(),
                    expected_loss: ExpectedLoss::High,
                    blocking: true,
                    reversible: false,
                    decide_by: None,
                    options: vec![DecisionOption {
                        id: "approve".into(),
                        label: "Approve once".into(),
                        recommended: true,
                    }],
                    default_option: Some("approve".into()),
                    related_call_key: None,
                },
            )],
        }));
        let content = render_text(&app, 110, 30);
        assert!(content.contains("Publish the release?"));
        assert!(content.contains("This changes the production service"));
        assert!(content.contains("Approve once"));
        assert!(content.contains("write a custom response"));
    }

    #[test]
    fn composer_and_confirmation_are_visible_at_narrow_sizes() {
        let mut app = app();
        app.screen = Screen::Composer;
        app.composer.insert_str("Ship a reliable terminal");
        let content = render_text(&app, 60, 20);
        assert!(content.contains("What outcome should Else own?"));
        assert!(content.contains("Ship a reliable terminal"));

        app.confirmation = Some(crate::tui::loader::ActionRequest::CreateThread {
            outcome: "unused".into(),
        });
        let content = render_text(&app, 60, 20);
        assert!(content.contains("Confirm approval"));
        assert!(content.contains("confirm approval"));
    }
}
