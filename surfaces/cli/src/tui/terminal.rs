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
