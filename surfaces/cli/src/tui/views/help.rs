//! 帮助浮层。

use super::super::theme;
use ratatui::text::{Line, Span};

pub fn lines() -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    let mut section = |title: &'static str, entries: &[(&'static str, &'static str)]| {
        lines.push(Line::from(Span::styled(
            format!(" {title}"),
            theme::heading(),
        )));
        for (key, description) in entries {
            lines.push(Line::from(vec![
                Span::styled(format!("   {key:<12}"), theme::hint_key()),
                Span::styled(*description, theme::text()),
            ]));
        }
        lines.push(Line::from(""));
    };
    section(
        "Navigate",
        &[
            ("j / k / ↑ / ↓", "move"),
            ("PgUp / PgDn", "move faster"),
            ("Home / End", "first / last row"),
            ("Enter", "open or choose"),
            ("Esc", "back"),
            ("mouse wheel", "scroll"),
        ],
    );
    section(
        "Surfaces",
        &[
            ("1", "Today"),
            ("2", "Decisions"),
            ("3", "Results"),
            ("4", "Settings / connectors"),
        ],
    );
    section(
        "Work",
        &[
            ("n", "new intent"),
            ("r", "refresh"),
            ("a", "advance an intent"),
            ("d", "toggle intent debug details"),
            ("e", "write a custom decision"),
            ("x", "dismiss a decision"),
            ("s", "sync a connector"),
        ],
    );
    section("Quit", &[("q", "quit"), ("Ctrl-C", "quit")]);
    lines.push(Line::from(Span::styled(
        "  press any key to close",
        theme::muted(),
    )));
    lines
}
