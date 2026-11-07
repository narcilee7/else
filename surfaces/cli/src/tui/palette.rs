//! 命令面板：`/` 打开的那一屏。
//!
//! 它是**页脚的展开**，不是另一套东西：这里出现的命令，就是此刻按对应键
//! 真的会发生的事。做不到的事不列进来 —— 面板不能承诺它执行不了的操作。

use super::app::{App, Screen};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    NewThread,
    OpenDecisions,
    OpenAllThreads,
    OpenConnections,
    Advance,
    Answer,
    Stop,
    ToggleDebug,
    Refresh,
    Help,
    Quit,
}

impl Command {
    /// 面板里那一行。
    pub fn title(self) -> &'static str {
        match self {
            Self::NewThread => "Describe a new thread",
            Self::OpenDecisions => "Review decisions",
            Self::OpenAllThreads => "See every thread",
            Self::OpenConnections => "Inspect connections",
            Self::Advance => "Let the runtime advance this thread",
            Self::Answer => "Write your own answer",
            Self::Stop => "Stop this run",
            Self::ToggleDebug => "Show debug detail",
            Self::Refresh => "Reload from the runtime",
            Self::Help => "All keys",
            Self::Quit => "Quit",
        }
    }

    /// 对应的键。面板的用处一半在"教会用户下次直接按"。
    pub fn key(self) -> &'static str {
        match self {
            Self::NewThread => "n",
            Self::OpenDecisions => "d",
            Self::OpenAllThreads => "—",
            Self::OpenConnections => "c",
            Self::Advance => "a",
            Self::Answer => "e",
            Self::Stop => "x",
            Self::ToggleDebug => "d",
            Self::Refresh => "r",
            Self::Help => "?",
            Self::Quit => "q",
        }
    }

    /// 这个词也用来匹配输入。
    fn matches(self, query: &str) -> bool {
        if query.is_empty() {
            return true;
        }
        let query = query.to_lowercase();
        self.title().to_lowercase().contains(&query)
            || self.key().to_lowercase().contains(&query)
            || format!("{self:?}").to_lowercase().contains(&query)
    }
}

/// 打开面板后累积的输入与选择。
#[derive(Debug, Clone, Default)]
pub struct Palette {
    pub query: String,
    pub cursor: usize,
}

impl Palette {
    pub fn filtered(&self, app: &App) -> Vec<Command> {
        available(app)
            .into_iter()
            .filter(|command| command.matches(&self.query))
            .collect()
    }
}

/// 此刻真的能执行的命令。
///
/// 判断与页脚共用 `App` 上的能力判断（`can_abort` 等），所以面板和页脚
/// 不会各说一套。
pub fn available(app: &App) -> Vec<Command> {
    let mut commands = vec![Command::NewThread, Command::Refresh];
    match app.screen {
        Screen::Overview => {
            commands.push(Command::OpenDecisions);
            commands.push(Command::OpenAllThreads);
            commands.push(Command::OpenConnections);
        }
        Screen::ThreadDetail => {
            if app.has_pending_decision() {
                commands.push(Command::Answer);
            }
            if app.can_advance() {
                commands.push(Command::Advance);
            }
            if app.can_abort() {
                commands.push(Command::Stop);
            }
            commands.push(Command::ToggleDebug);
            commands.push(Command::OpenDecisions);
        }
        Screen::Decisions | Screen::DecisionDetail => {
            commands.push(Command::OpenAllThreads);
        }
        _ => {}
    }
    commands.push(Command::Help);
    commands.push(Command::Quit);
    commands
}
