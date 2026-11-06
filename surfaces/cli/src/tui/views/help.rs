//! 帮助浮层：按 `?` 才出现的那张卡。
//!
//! 页脚只放此刻用得上的键，其余的都在这里 —— 而这里只写真的存在的键。

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
        "Moving",
        &[
            ("j / k / ↑ / ↓", "one row"),
            ("PgUp / PgDn", "ten rows"),
            ("Home / End", "first / last"),
            ("Enter", "open or choose"),
            ("Esc", "go back"),
        ],
    );
    section(
        "Working",
        &[
            ("n", "describe a new thread"),
            ("a", "let the runtime advance"),
            ("e", "write your own answer"),
            ("x", "dismiss a request"),
            ("d", "decisions · debug (in a thread)"),
            ("c", "connections"),
            ("r", "reload"),
        ],
    );
    section("Leaving", &[("q", "quit"), ("Ctrl-C", "quit")]);
    lines.push(Line::from(Span::styled(
        "  press any key to close",
        theme::muted(),
    )));
    lines
}
