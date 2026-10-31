//! 终端配色。
//!
//! 配色按**语义**走，不按界面走：同一个状态在任何屏幕上都是同一个颜色。
//! 紧急的事必须是红的，等一等是蓝的，做完了是绿的 —— 用户不该为了看懂
//! 界面而学习一套新的视觉语言。

use ratatui::style::{Color, Modifier, Style};

/// 品牌强调色：标题、当前选中项。
pub const ACCENT: Color = Color::Cyan;

/// 紧急：逾期、需要立刻处理。
pub const CRITICAL: Color = Color::Red;

/// 风险：还没有逾期，但正在变坏。
pub const WARNING: Color = Color::Yellow;

/// 等待外部：不是问题，是等人。
pub const WAITING: Color = Color::Blue;

/// 已完成、已收敛。
pub const DONE: Color = Color::Green;

/// 次要信息：元数据、计数。
pub const MUTED: Color = Color::DarkGray;

/// 普通正文。
pub const TEXT: Color = Color::Gray;

/// 选中项的样式。
pub fn selected() -> Style {
    Style::default()
        .fg(Color::Black)
        .bg(ACCENT)
        .add_modifier(Modifier::BOLD)
}

/// 未选中但当前焦点所在项。
pub fn focused() -> Style {
    Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)
}

/// 小标题。
pub fn heading() -> Style {
    Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)
}

/// 分组标题。
pub fn section() -> Style {
    Style::default()
        .fg(Color::White)
        .add_modifier(Modifier::BOLD)
}

/// 次要文本。
pub fn muted() -> Style {
    Style::default().fg(MUTED)
}

/// 普通文本。
pub fn text() -> Style {
    Style::default().fg(TEXT)
}

/// 紧急文本。
pub fn critical() -> Style {
    Style::default().fg(CRITICAL).add_modifier(Modifier::BOLD)
}

/// 风险文本。
pub fn warning() -> Style {
    Style::default().fg(WARNING)
}

/// 等待中的文本。
pub fn waiting() -> Style {
    Style::default().fg(WAITING)
}

/// 已完成的文本。
pub fn done() -> Style {
    Style::default().fg(DONE)
}

/// 底部提示。
pub fn hint() -> Style {
    Style::default().fg(MUTED)
}

/// 底部提示里的按键。
pub fn hint_key() -> Style {
    Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)
}
