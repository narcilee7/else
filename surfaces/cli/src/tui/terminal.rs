//! 终端接管与还原。
//!
//! 界面崩了、用户按了 Ctrl-C、进程被杀掉 —— 终端都必须回到原样。
//! 这里用 RAII 保证：只要 `Tui` 被 drop，raw mode 与备用屏幕就会被还原。

use std::io::{Stdout, Write};

use crossterm::event::{
    DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

/// 一个已接管终端的 TUI。
pub struct Tui {
    terminal: Terminal<CrosstermBackend<Stdout>>,
}

impl Tui {
    /// 接管终端。
    pub fn enter() -> std::io::Result<Self> {
        enable_raw_mode()?;
        let mut stdout = std::io::stdout();
        execute!(
            stdout,
            EnterAlternateScreen,
            EnableMouseCapture,
            EnableBracketedPaste
        )?;
        let terminal = Terminal::new(CrosstermBackend::new(stdout))?;
        Ok(Self { terminal })
    }

    /// 画一帧。
    pub fn draw<F>(&mut self, render: F) -> std::io::Result<()>
    where
        F: FnOnce(&mut ratatui::Frame<'_>),
    {
        self.terminal.draw(render).map(|_| ())
    }

    /// 发一条终端原生通知。
    ///
    /// 只往认得的终端里发：识别不了就不发，也不要往未知终端塞转义序列把
    /// 屏幕搞花。这就是 herdr 在 `terminal_notify.rs` 里的做法。
    pub fn notify(&mut self, title: &str, body: &str) -> std::io::Result<()> {
        let Some(sequence) = notification_sequence(title, body) else {
            return Ok(());
        };
        let out = self.terminal.backend_mut();
        out.write_all(sequence.as_bytes())?;
        out.flush()
    }

    /// 改窗口标题。用户切到别的标签页时，标题是唯一还在的提醒。
    pub fn set_title(&mut self, title: &str) -> std::io::Result<()> {
        execute!(
            self.terminal.backend_mut(),
            crossterm::terminal::SetTitle(title)
        )
    }
}

/// 哪些终端支持哪种通知序列。识别不出来就没法保证不画花。
fn notification_sequence(title: &str, body: &str) -> Option<String> {
    let program = std::env::var("TERM_PROGRAM").unwrap_or_default();
    match program.as_str() {
        "iTerm.app" => Some(format!("\u{1b}]9;{title}\n{body}\u{7}")),
        "ghostty" | "WezTerm" | "kitty" => Some(format!("\u{1b}]9;{body}\u{7}")),
        _ => None,
    }
}

impl Drop for Tui {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(
            self.terminal.backend_mut(),
            LeaveAlternateScreen,
            DisableMouseCapture,
            DisableBracketedPaste
        );
        let _ = self.terminal.show_cursor();
        let _ = std::io::stdout().flush();
    }
}

#[cfg(test)]
mod tests {
    use super::notification_sequence;

    #[test]
    fn notifications_only_go_to_terminals_we_can_recognise() {
        // 环境变量在测试之间共享；逐个设置、读完就还原。
        let previous = std::env::var("TERM_PROGRAM").ok();
        std::env::remove_var("TERM_PROGRAM");
        assert_eq!(notification_sequence("else", "done"), None);

        std::env::set_var("TERM_PROGRAM", "ghostty");
        let sequence = notification_sequence("else", "1 thing needs you").expect("osc 9");
        assert!(sequence.starts_with("\u{1b}]9;"));
        assert!(sequence.ends_with('\u{7}'));
        assert!(sequence.contains("1 thing needs you"));

        std::env::set_var("TERM_PROGRAM", "iTerm.app");
        let sequence = notification_sequence("else", "body").expect("iterm");
        assert!(sequence.contains("else\nbody"));

        match previous {
            Some(value) => std::env::set_var("TERM_PROGRAM", value),
            None => std::env::remove_var("TERM_PROGRAM"),
        }
    }
}
