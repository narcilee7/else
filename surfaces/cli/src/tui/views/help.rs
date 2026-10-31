//! 帮助浮层。

use ratatui::text::{Line, Span};

use super::super::theme;

/// 键位说明。
pub fn lines() -> Vec<Line<'static>> {
    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut section = |title: &'static str, entries: &[(&'static str, &'static str)]| {
        lines.push(Line::from(Span::styled(
            format!(" {title}"),
            theme::heading(),
        )));
        for (key, description) in entries {
            lines.push(Line::from(vec![
                Span::styled(format!("   {key:<10}"), theme::hint_key()),
                Span::styled(*description, theme::text()),
            ]));
        }
        lines.push(Line::from(""));
    };
    section(
        "Navigate",
        &[
            ("j / k", "move down / up"),
            ("↑ / ↓", "move down / up"),
            ("PgUp/PgDn", "move a page"),
            ("Home/End", "jump to first / last"),
            ("Enter", "open the selected row"),
            ("Esc", "clear the focused intent"),
        ],
    );
    section(
        "Screens",
        &[
            ("1", "Overview — what needs you today"),
            ("2", "Intents"),
            ("3", "World"),
            ("4", "Decisions"),
            ("5", "Connectors"),
        ],
    );
    section(
        "Act",
        &[
            ("n", "new intent"),
            ("r", "refresh"),
            ("a", "approve (Decisions) / advance (Intent)"),
            ("r", "reject (Decisions)"),
            ("s", "defer (Decisions) / sync (Connectors)"),
            ("c", "complete a World item"),
            ("d", "dismiss a World item"),
        ],
    );
    section("Quit", &[("q", "quit"), ("Ctrl-C", "quit")]);
    lines.push(Line::from(Span::styled(
        "  press any key to close",
        theme::muted(),
    )));
    lines
}
