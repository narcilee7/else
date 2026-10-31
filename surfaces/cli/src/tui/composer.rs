//! 单行文本输入。
//!
//! 自己写而不用 `tui-textarea`：那个 crate 停在 2024-10，没跟上 ratatui 0.30；
//! Codex 与 Grok 也都是自己写的。这里只需要单行输入，不必背一个编辑器。

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// 一行正在编辑的文本。
#[derive(Debug, Default, Clone)]
pub struct Composer {
    buffer: String,
    /// 光标位置，按字符计数。
    cursor: usize,
}

impl Composer {
    /// 空输入框。
    pub fn new() -> Self {
        Self::default()
    }

    /// 当前文本。
    pub fn text(&self) -> &str {
        &self.buffer
    }

    /// 清空。
    pub fn clear(&mut self) {
        self.buffer.clear();
        self.cursor = 0;
    }

    /// 吞掉一个按键。返回 `true` 表示这一键被输入框消费了。
    pub fn handle(&mut self, key: KeyEvent) -> bool {
        match key.code {
            KeyCode::Char(character) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.insert(character);
                true
            }
            KeyCode::Backspace => {
                self.delete_backward();
                true
            }
            KeyCode::Delete => {
                self.delete_forward();
                true
            }
            KeyCode::Left => {
                self.cursor = self.cursor.saturating_sub(1);
                true
            }
            KeyCode::Right => {
                self.cursor = (self.cursor + 1).min(self.buffer.chars().count());
                true
            }
            KeyCode::Home => {
                self.cursor = 0;
                true
            }
            KeyCode::End => {
                self.cursor = self.buffer.chars().count();
                true
            }
            _ => false,
        }
    }

    fn insert(&mut self, character: char) {
        let index = self.byte_index(self.cursor);
        self.buffer.insert(index, character);
        self.cursor += 1;
    }

    fn delete_backward(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let index = self.byte_index(self.cursor - 1);
        self.buffer.remove(index);
        self.cursor -= 1;
    }

    fn delete_forward(&mut self) {
        let count = self.buffer.chars().count();
        if self.cursor >= count {
            return;
        }
        let index = self.byte_index(self.cursor);
        self.buffer.remove(index);
    }

    /// 字符位置对应的字节位置。
    fn byte_index(&self, char_index: usize) -> usize {
        self.buffer
            .char_indices()
            .nth(char_index)
            .map(|(index, _)| index)
            .unwrap_or(self.buffer.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn composer_inserts_and_deletes() {
        let mut composer = Composer::new();
        composer.handle(key(KeyCode::Char('a')));
        composer.handle(key(KeyCode::Char('中')));
        assert_eq!(composer.text(), "a中");

        composer.handle(key(KeyCode::Backspace));
        assert_eq!(composer.text(), "a");
        composer.handle(key(KeyCode::Backspace));
        assert_eq!(composer.text(), "");
    }

    #[test]
    fn composer_moves_the_cursor_and_inserts_at_the_right_place() {
        let mut composer = Composer::new();
        composer.handle(key(KeyCode::Char('a')));
        composer.handle(key(KeyCode::Char('b')));
        composer.handle(key(KeyCode::Left));
        composer.handle(key(KeyCode::Char('x')));
        assert_eq!(composer.text(), "axb");
    }

    #[test]
    fn composer_ignores_control_shortcuts() {
        let mut composer = Composer::new();
        let control_c = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
        assert!(!composer.handle(control_c));
        assert_eq!(composer.text(), "");
    }
}
