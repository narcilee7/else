//! 屏幕。
//!
//! 每个屏幕把 Runtime 的数据摊成**若干面板**，面板里是一行行内容。
//! 渲染器只负责把面板画出来、把光标标出来 —— 屏幕本身不碰 ratatui，
//! 也不关心终端有多宽。

pub mod connectors;
pub mod decisions;
pub mod help;
pub mod intent_detail;
pub mod intents;
pub mod overview;
pub mod world;

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

use super::app::App;
use super::data::Data;
use super::row::{Pane, Row};
use super::theme;

/// 当前屏幕的面板。
pub fn panes(app: &App) -> Vec<Pane> {
    match &app.data {
        Data::Overview(data) => vec![overview::attention_pane(data), overview::world_pane(data)],
        Data::Intents(intents) => vec![intents::pane(intents)],
        Data::IntentDetail(detail) => vec![intent_detail::pane(detail)],
        Data::World(items) => vec![world::pane(items, app.focused_intent.as_ref())],
        Data::Decisions(windows) => vec![decisions::pane(windows)],
        Data::Connectors(rows) => vec![connectors::pane(rows, app.focused_intent.as_ref())],
        Data::Empty => vec![Pane {
            title: app.screen.title().to_string(),
            rows: vec![Row::muted("loading…")],
        }],
    }
}

/// 拍平所有面板的行：光标在这个序列上移动。
pub fn flat_rows(app: &App) -> Vec<Row> {
    panes(app).into_iter().flat_map(|pane| pane.rows).collect()
}

/// 画一帧。
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
    if app.screen == super::app::Screen::Composer {
        draw_composer(frame, areas[1], app);
    } else {
        draw_body(frame, areas[1], app);
    }
    draw_footer(frame, areas[2], app);
    if app.show_help {
        draw_help(frame, frame.area());
    }
}

/// 顶部：标题与屏幕切换。
fn draw_header(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let mut tabs = Vec::new();
    for (index, (screen, label)) in [
        (super::app::Screen::Overview, "1 Overview"),
        (super::app::Screen::Intents, "2 Intents"),
        (super::app::Screen::World, "3 World"),
        (super::app::Screen::Decisions, "4 Decisions"),
        (super::app::Screen::Connectors, "5 Connectors"),
    ]
    .iter()
    .enumerate()
    {
        if index > 0 {
            tabs.push(Span::styled("  ", theme::muted()));
        }
        let style = if *screen == app.screen {
            theme::focused()
        } else {
            theme::hint()
        };
        tabs.push(Span::styled(*label, style));
    }
    let title = Line::from(vec![
        Span::styled("else ", theme::heading()),
        Span::styled("you do what matters", theme::muted()),
    ]);
    let tabs = Line::from(tabs);
    let block = Block::default().borders(Borders::ALL).title(title);
    frame.render_widget(Paragraph::new(tabs).block(block), area);
}

/// 主体：面板。
fn draw_body(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let panes = panes(app);
    if panes.len() >= 2 {
        let columns = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(area);
        let mut offset = 0usize;
        for (index, pane) in panes.iter().take(2).enumerate() {
            draw_pane(frame, columns[index], pane, offset, app.cursor);
            offset += pane.rows.len();
        }
        return;
    }
    let pane = panes.first().cloned().unwrap_or(Pane {
        title: app.screen.title().to_string(),
        rows: vec![Row::muted("nothing to show")],
    });
    draw_pane(frame, area, &pane, 0, app.cursor);
}

/// 画一个面板。
fn draw_pane(frame: &mut Frame<'_>, area: Rect, pane: &Pane, offset: usize, cursor: usize) {
    let visible = area.height.saturating_sub(2) as usize;
    // 让选中项尽量落在可视区中间偏上。
    let selected = cursor.saturating_sub(offset);
    let scroll = if visible == 0 || selected < visible / 2 {
        0
    } else {
        selected - visible / 2
    };
    let lines: Vec<Line> = pane
        .rows
        .iter()
        .enumerate()
        .skip(scroll)
        .take(visible)
        .map(|(index, row)| {
            let style: Style = if offset + index == cursor {
                theme::selected()
            } else {
                row.style
            };
            Line::from(Span::styled(row.text.clone(), style))
        })
        .collect();
    let block = Block::default()
        .borders(Borders::ALL)
        .title(Span::styled(format!(" {} ", pane.title), theme::heading()));
    frame.render_widget(Paragraph::new(lines).block(block), area);
}

/// 新建 Intent 的输入框。
fn draw_composer(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let text = app.composer.text();
    let mut spans = vec![Span::styled(text.to_string(), theme::text())];
    // 光标：一个反色块。
    spans.push(Span::styled("█", theme::selected()));
    let paragraph = Paragraph::new(Line::from(spans)).block(
        Block::default()
            .borders(Borders::ALL)
            .title(Span::styled(" what should happen? ", theme::heading())),
    );
    frame.render_widget(paragraph, area);
    let hint = Paragraph::new(Line::from(vec![
        Span::styled("  Enter", theme::hint_key()),
        Span::styled(" create  ", theme::hint()),
        Span::styled("Esc", theme::hint_key()),
        Span::styled(" cancel", theme::hint()),
    ]));
    let hint_area = Rect {
        x: area.x,
        y: area.y + area.height.saturating_sub(1),
        width: area.width,
        height: 1,
    };
    frame.render_widget(hint, hint_area);
}

/// 底部：按键提示与状态。
fn draw_footer(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let mut hints = vec![
        Span::styled("j/k", theme::hint_key()),
        Span::styled(" move  ", theme::hint()),
        Span::styled("1-5", theme::hint_key()),
        Span::styled(" screens  ", theme::hint()),
        Span::styled("n", theme::hint_key()),
        Span::styled(" new  ", theme::hint()),
        Span::styled("r", theme::hint_key()),
        Span::styled(" refresh  ", theme::hint()),
        Span::styled("?", theme::hint_key()),
        Span::styled(" help  ", theme::hint()),
        Span::styled("q", theme::hint_key()),
        Span::styled(" quit", theme::hint()),
    ];
    if let Some(status) = &app.status {
        let style = if status.failed {
            theme::critical()
        } else {
            theme::done()
        };
        hints.push(Span::styled("  │  ", theme::muted()));
        hints.push(Span::styled(status.text.clone(), style));
    }
    frame.render_widget(Paragraph::new(Line::from(hints)), area);
}

/// 帮助浮层。
pub fn draw_help(frame: &mut Frame<'_>, area: Rect) {
    let width = area.width.saturating_sub(8).min(72);
    let height = area.height.saturating_sub(4).min(24);
    let popup = Rect {
        x: area.x + (area.width.saturating_sub(width)) / 2,
        y: area.y + (area.height.saturating_sub(height)) / 2,
        width,
        height,
    };
    frame.render_widget(ratatui::widgets::Clear, popup);
    let lines = help::lines();
    let block = Block::default()
        .borders(Borders::ALL)
        .title(Span::styled(" keys ", theme::heading()));
    frame.render_widget(Paragraph::new(lines).block(block), popup);
}
