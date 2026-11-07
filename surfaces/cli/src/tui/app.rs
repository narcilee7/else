//! 顶层应用：导航、页面状态、输入分发与异步结果协调。

use std::sync::mpsc::{Receiver, Sender};
use std::time::Duration;

use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseEventKind};
use else_protocol::attention::{DecisionChoice, ExpectedLoss};
use else_protocol::ids::{AttentionId, ThreadId};

use super::app_event::AppEvent;
use super::composer::Composer;
use super::data::Data;
use super::loader::{ActionRequest, LoadRequest};
use super::palette::{Command, Palette};
use super::row::{Row, RowAction};
use super::terminal::Tui;
use super::views;

/// 当前屏幕。Today / Decisions / Results 是主表面，其余是工作流页面。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    Overview,
    Decisions,
    Results,
    ThreadDetail,
    DecisionDetail,
    Connectors,
    Composer,
}

#[derive(Debug, Clone)]
pub struct DecisionFocus {
    pub thread_id: ThreadId,
    pub request_id: AttentionId,
}

#[derive(Debug, Clone)]
pub enum ComposerMode {
    Thread,
    Decision {
        thread_id: ThreadId,
        request_id: AttentionId,
    },
}

/// 底部状态条。
#[derive(Debug, Clone)]
pub struct Status {
    pub text: String,
    pub failed: bool,
}

pub struct App {
    pub screen: Screen,
    pub data: Data,
    pub cursor: usize,
    pub status: Option<Status>,
    pub composer: Composer,
    pub composer_mode: ComposerMode,
    pub generation: u64,
    inflight: Option<u64>,
    refresh_again: bool,
    last_refresh: std::time::Instant,
    pub dirty: bool,
    pub should_quit: bool,
    /// 页脚常驻提示：这条命令接的是常驻 Runtime 还是临时替身。
    pub runtime_note: Option<String>,
    /// 最近一次看到"有几件事在等人"。写进终端标题，切走窗口也不会错过。
    waiting: usize,
    /// 刚提交的是"开一件事"：做完就带我去看它，不要把我丢回列表。
    open_after_create: bool,
    /// 已经见过的待决请求 id。只对**新来的**发通知，不重复打扰。
    known_requests: std::collections::BTreeSet<String>,
    /// 排队中的系统通知。App 不碰终端 IO，由 `run` 取走。
    pub notification: Option<String>,
    /// `/` 打开的命令面板。
    pub palette: Option<Palette>,
    pub show_help: bool,
    pub focused_thread: Option<ThreadId>,
    pub focused_decision: Option<DecisionFocus>,
    pub debug_detail: bool,
    pub action_pending: bool,
    pub confirmation: Option<ActionRequest>,
    history: Vec<Screen>,
    loads: Sender<LoadRequest>,
    actions: Sender<ActionRequest>,
}

impl App {
    pub fn new(loads: Sender<LoadRequest>, actions: Sender<ActionRequest>) -> Self {
        Self {
            screen: Screen::Overview,
            data: Data::Empty,
            cursor: 0,
            status: None,
            composer: Composer::new(),
            composer_mode: ComposerMode::Thread,
            generation: 0,
            inflight: None,
            refresh_again: false,
            last_refresh: std::time::Instant::now(),
            dirty: true,
            should_quit: false,
            runtime_note: None,
            waiting: 0,
            open_after_create: false,
            known_requests: std::collections::BTreeSet::new(),
            notification: None,
            palette: None,
            show_help: false,
            focused_thread: None,
            focused_decision: None,
            debug_detail: false,
            action_pending: false,
            confirmation: None,
            history: Vec::new(),
            loads,
            actions,
        }
    }

    pub fn rows(&self) -> Vec<Row> {
        views::flat_rows(self)
    }

    /// 终端标签页标题。人不在这个窗口里的时候，这里就是通知。
    pub fn window_title(&self) -> String {
        if self.waiting > 0 {
            return format!("[!] {} waiting · else", self.waiting);
        }
        "else".to_string()
    }

    pub fn is_loading(&self) -> bool {
        self.inflight.is_some()
    }

    /// 使当前页面的数据失效并请求新代次。导航期间旧响应可以完成，但不能覆盖新页面。
    pub fn request_load(&mut self) {
        if self.screen == Screen::Composer {
            return;
        }
        self.generation += 1;
        if self.inflight.is_some() {
            self.refresh_again = true;
            return;
        }
        self.refresh_again = false;
        self.send_load();
    }

    fn send_load(&mut self) {
        let generation = self.generation;
        let request = match self.screen {
            Screen::Overview | Screen::Composer => LoadRequest::Overview { generation },
            Screen::Results => LoadRequest::Threads { generation },
            Screen::ThreadDetail => match self.focused_thread.clone() {
                Some(thread_id) => LoadRequest::ThreadDetail {
                    generation,
                    thread_id,
                },
                None => LoadRequest::Threads { generation },
            },
            Screen::Decisions | Screen::DecisionDetail => LoadRequest::Decisions { generation },
            Screen::Connectors => match self.focused_thread.clone() {
                Some(thread_id) => LoadRequest::Connectors {
                    generation,
                    thread_id,
                },
                None => LoadRequest::Threads { generation },
            },
        };
        self.inflight = Some(generation);
        self.last_refresh = std::time::Instant::now();
        if self.loads.send(request).is_err() {
            self.inflight = None;
            self.status = Some(Status {
                text: "The loader is unavailable. Restart the terminal interface.".into(),
                failed: true,
            });
        }
        self.dirty = true;
    }

    pub fn tick(&mut self) {
        if !self.refresh_again || self.inflight.is_some() {
            return;
        }
        if self.last_refresh.elapsed() < Duration::from_millis(750) {
            return;
        }
        self.generation += 1;
        self.refresh_again = false;
        self.send_load();
    }

    pub fn goto(&mut self, screen: Screen) {
        if self.screen == screen {
            return;
        }
        self.history.push(self.screen);
        self.screen = screen;
        self.cursor = 0;
        self.debug_detail = false;
        if screen != Screen::Composer && !Self::data_fits(screen, &self.data) {
            self.data = Data::Empty;
        }
        self.dirty = true;
        self.request_load();
    }

    pub fn back(&mut self) {
        let screen = self.history.pop().unwrap_or(Screen::Overview);
        self.screen = screen;
        self.cursor = 0;
        self.confirmation = None;
        self.debug_detail = false;
        if !Self::data_fits(screen, &self.data) {
            self.data = Data::Empty;
        }
        self.dirty = true;
        self.request_load();
    }

    fn data_fits(screen: Screen, data: &Data) -> bool {
        match screen {
            Screen::Overview => matches!(data, Data::Overview(_)),
            Screen::Results => matches!(data, Data::Threads(_)),
            Screen::ThreadDetail => matches!(data, Data::ThreadDetail(_) | Data::Threads(_)),
            Screen::Decisions | Screen::DecisionDetail => matches!(data, Data::Decisions(_)),
            Screen::Connectors => matches!(data, Data::Connectors(_) | Data::Threads(_)),
            Screen::Composer => true,
        }
    }

    pub fn handle(&mut self, event: AppEvent) {
        match event {
            AppEvent::Resize => self.dirty = true,
            AppEvent::Loaded { generation, data } => {
                if self.inflight != Some(generation) {
                    return;
                }
                self.inflight = None;
                if generation != self.generation || !Self::data_fits(self.screen, &data) {
                    self.refresh_again = true;
                    self.dirty = true;
                    return;
                }
                let selected = self.selected_action().map(|action| action.stable_key());
                if let Data::Overview(overview) = &data {
                    // 首屏之外的行没有总账；把"有多少事在等人"记在 App 上，
                    // 切到 Thread 时窗口标题仍然说实话。
                    self.waiting = views::waiting_count(overview);
                    self.track_new_requests(overview);
                }
                self.data = data;
                if let Some(selected) = selected {
                    if let Some(index) = self.rows().iter().position(|row| {
                        row.action
                            .as_ref()
                            .is_some_and(|action| action.stable_key() == selected)
                    }) {
                        self.cursor = index;
                    }
                }
                self.clamp_cursor();
                if self.selected_action().is_none() {
                    if let Some(index) = self.rows().iter().position(|row| row.action.is_some()) {
                        self.cursor = index;
                    }
                }
                self.dirty = true;
                // Runtime 事件可能在本次请求期间到达；不能在这里吞掉下一轮刷新。
            }
            AppEvent::LoadFailed { generation, error } => {
                if self.inflight != Some(generation) {
                    return;
                }
                self.inflight = None;
                if generation != self.generation {
                    self.refresh_again = true;
                } else {
                    self.status = Some(Status {
                        text: format!("{error} · press r to retry"),
                        failed: true,
                    });
                }
                self.dirty = true;
            }
            AppEvent::RuntimeEvent(event) => {
                let _ = event.seq;
                self.refresh_again = true;
            }
            AppEvent::ActionDone {
                label,
                detail,
                thread_id,
            } => {
                self.action_pending = false;
                self.status = Some(Status {
                    text: format!("{label} · {detail}"),
                    failed: false,
                });
                if let Some(thread_id) = thread_id {
                    self.focused_thread = Some(thread_id);
                }
                if self.open_after_create {
                    // 新建之后直接进那条 Thread：你刚说要做的事，就是你现在想看的事。
                    self.open_after_create = false;
                    if self.focused_thread.is_some() {
                        self.goto(Screen::ThreadDetail);
                    }
                } else if self.screen == Screen::DecisionDetail {
                    self.back();
                }
                self.refresh_again = true;
                self.dirty = true;
            }
            AppEvent::ActionFailed { error } => {
                self.action_pending = false;
                self.status = Some(Status {
                    text: error,
                    failed: true,
                });
                self.dirty = true;
            }
        }
    }

    pub fn handle_key(&mut self, key: KeyEvent) {
        if key.kind != KeyEventKind::Press {
            return;
        }
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            self.should_quit = true;
            return;
        }
        if self.confirmation.is_some() {
            match key.code {
                KeyCode::Enter => {
                    if let Some(action) = self.confirmation.take() {
                        self.dispatch(action);
                    }
                }
                KeyCode::Esc => {
                    self.confirmation = None;
                    self.status = Some(Status {
                        text: "Approval cancelled".into(),
                        failed: false,
                    });
                    self.dirty = true;
                }
                _ => {}
            }
            return;
        }
        if self.show_help {
            self.show_help = false;
            self.dirty = true;
            return;
        }
        if self.palette.is_some() {
            self.handle_palette_key(key);
            return;
        }
        if self.screen == Screen::Composer {
            self.handle_composer_key(key);
            return;
        }
        match key.code {
            KeyCode::Char('q') => {
                self.should_quit = true;
                return;
            }
            KeyCode::Char('?') => {
                self.show_help = true;
                self.dirty = true;
                return;
            }
            KeyCode::Char('/') => {
                self.palette = Some(Palette::default());
                self.dirty = true;
                return;
            }
            // 不做数字标签页：屏幕不是并列的功能模块，是任务的不同深度。
            KeyCode::Char('d') if self.screen != Screen::ThreadDetail => {
                self.goto(Screen::Decisions)
            }
            KeyCode::Char('c') => self.goto(Screen::Connectors),
            KeyCode::Char('r') => self.request_load(),
            KeyCode::Char('n') => {
                self.composer.clear();
                self.composer_mode = ComposerMode::Thread;
                self.goto(Screen::Composer);
                return;
            }
            KeyCode::Down | KeyCode::Char('j') => self.move_cursor(1),
            KeyCode::Up | KeyCode::Char('k') => self.move_cursor(-1),
            KeyCode::PageDown => self.move_cursor(10),
            KeyCode::PageUp => self.move_cursor(-10),
            KeyCode::Home => self.set_cursor(0),
            KeyCode::End => self.set_cursor(usize::MAX),
            KeyCode::Enter => self.activate(),
            KeyCode::Esc => self.back(),
            _ => self.screen_key(key),
        }
        self.dirty = true;
    }

    fn screen_key(&mut self, key: KeyEvent) {
        match self.screen {
            Screen::DecisionDetail => match key.code {
                KeyCode::Char('e') => {
                    if let Some(focus) = self.focused_decision.clone() {
                        self.composer.clear();
                        self.composer_mode = ComposerMode::Decision {
                            thread_id: focus.thread_id,
                            request_id: focus.request_id,
                        };
                        self.goto(Screen::Composer);
                    }
                }
                KeyCode::Char('x') => {
                    if let Some(focus) = self.focused_decision.clone() {
                        self.dispatch(ActionRequest::Decide {
                            thread_id: focus.thread_id,
                            request_id: focus.request_id,
                            choice: DecisionChoice::Dismissed,
                        });
                    }
                }
                _ => {}
            },
            Screen::ThreadDetail => match key.code {
                KeyCode::Char('a') => {
                    if let Some(thread_id) = self.focused_thread.clone() {
                        self.dispatch(ActionRequest::AdvanceThread { thread_id });
                    }
                }
                // 时间线上待决的事就地回答，不必先退回去找那条请求。
                KeyCode::Char('e') => {
                    if let Some((thread_id, request_id)) = self.pending_decision() {
                        self.composer.clear();
                        self.composer_mode = ComposerMode::Decision {
                            thread_id,
                            request_id,
                        };
                        self.goto(Screen::Composer);
                    }
                }
                // 跑飞了必须能就地停住；收尾之后这个键不存在。
                KeyCode::Char('x') if self.can_abort() => {
                    if let Some(thread_id) = self.focused_thread.clone() {
                        let action = ActionRequest::AbortThread {
                            thread_id,
                            reason: "stopped from the terminal".into(),
                        };
                        if self.requires_confirmation(&action) {
                            self.confirmation = Some(action);
                        } else {
                            self.dispatch(action);
                        }
                    }
                }
                KeyCode::Char('d') => self.debug_detail = !self.debug_detail,
                _ => {}
            },
            Screen::Connectors if key.code == KeyCode::Char('s') => {
                if let Some(RowAction::SyncConnector {
                    thread_id,
                    connector_id,
                }) = self.selected_action()
                {
                    self.dispatch(ActionRequest::SyncConnector {
                        thread_id,
                        connector_id,
                    });
                }
            }
            _ => {}
        }
    }

    fn handle_palette_key(&mut self, key: KeyEvent) {
        // 先把面板取出来：匹配要读整个 App（当前屏幕决定了能做什么），
        // 而它在 self 里。
        let Some(mut palette) = self.palette.take() else {
            return;
        };
        self.dirty = true;
        match key.code {
            KeyCode::Esc => return,
            KeyCode::Enter => {
                if let Some(command) = palette.filtered(self).get(palette.cursor).copied() {
                    self.run_command(command);
                }
                return;
            }
            KeyCode::Down | KeyCode::Tab => {
                let count = palette.filtered(self).len();
                palette.cursor = next_index(palette.cursor, count, 1);
            }
            KeyCode::Up | KeyCode::BackTab => {
                let count = palette.filtered(self).len();
                palette.cursor = next_index(palette.cursor, count, -1);
            }
            KeyCode::Backspace => {
                palette.query.pop();
                palette.cursor = 0;
            }
            KeyCode::Char(character) => {
                palette.query.push(character);
                palette.cursor = 0;
            }
            _ => {}
        }
        palette.cursor = palette
            .cursor
            .min(palette.filtered(self).len().saturating_sub(1));
        self.palette = Some(palette);
    }

    /// 执行一条面板命令。每一条都走和快捷键相同的那条路。
    fn run_command(&mut self, command: Command) {
        match command {
            Command::NewThread => {
                self.composer.clear();
                self.composer_mode = ComposerMode::Thread;
                self.goto(Screen::Composer);
            }
            Command::OpenDecisions => self.goto(Screen::Decisions),
            Command::OpenAllThreads => self.goto(Screen::Results),
            Command::OpenConnections => self.goto(Screen::Connectors),
            Command::Advance => {
                if let Some(thread_id) = self.focused_thread.clone() {
                    self.dispatch(ActionRequest::AdvanceThread { thread_id });
                }
            }
            Command::Answer => {
                if let Some((thread_id, request_id)) = self.pending_decision() {
                    self.composer.clear();
                    self.composer_mode = ComposerMode::Decision {
                        thread_id,
                        request_id,
                    };
                    self.goto(Screen::Composer);
                }
            }
            Command::Stop => {
                if let Some(thread_id) = self.focused_thread.clone() {
                    self.confirmation = Some(ActionRequest::AbortThread {
                        thread_id,
                        reason: "stopped from the command palette".into(),
                    });
                }
            }
            Command::ToggleDebug => self.debug_detail = !self.debug_detail,
            Command::Refresh => self.request_load(),
            Command::Help => self.show_help = true,
            Command::Quit => self.should_quit = true,
        }
        self.dirty = true;
    }

    fn handle_composer_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => {
                self.back();
                return;
            }
            KeyCode::Enter => {
                let text = self.composer.text().trim().to_string();
                if text.is_empty() {
                    self.status = Some(Status {
                        text: "Write something before submitting".into(),
                        failed: true,
                    });
                    self.dirty = true;
                    return;
                }
                let action = match &self.composer_mode {
                    ComposerMode::Thread => ActionRequest::CreateThread { outcome: text },
                    ComposerMode::Decision {
                        thread_id,
                        request_id,
                    } => ActionRequest::Decide {
                        thread_id: thread_id.clone(),
                        request_id: request_id.clone(),
                        choice: DecisionChoice::FreeText(text),
                    },
                };
                self.open_after_create = matches!(action, ActionRequest::CreateThread { .. });
                self.dispatch(action);
                self.back();
                return;
            }
            _ => {}
        }
        if self.composer.handle(key) {
            self.dirty = true;
        }
    }

    pub fn handle_paste(&mut self, text: &str) {
        if self.screen != Screen::Composer {
            return;
        }
        let text = text
            .replace(['\r', '\n'], " ")
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        self.composer.insert_str(&text);
        self.dirty = true;
    }

    fn dispatch(&mut self, action: ActionRequest) {
        if self.action_pending {
            self.status = Some(Status {
                text: "Another action is still running".into(),
                failed: true,
            });
            self.dirty = true;
            return;
        }
        self.action_pending = true;
        self.status = Some(Status {
            text: action.pending_label().into(),
            failed: false,
        });
        if self.actions.send(action).is_err() {
            self.action_pending = false;
            self.status = Some(Status {
                text: "The action worker is unavailable. Restart the terminal interface.".into(),
                failed: true,
            });
        }
        self.dirty = true;
    }

    fn activate(&mut self) {
        let Some(action) = self.selected_action() else {
            return;
        };
        match action {
            RowAction::OpenThread(thread_id) => {
                self.focused_thread = Some(thread_id);
                self.goto(Screen::ThreadDetail);
            }
            RowAction::OpenResults => self.goto(Screen::Results),
            RowAction::OpenDecision {
                thread_id,
                request_id,
            } => {
                self.focused_decision = Some(DecisionFocus {
                    thread_id,
                    request_id,
                });
                self.goto(Screen::DecisionDetail);
            }
            RowAction::OpenConnectors(thread_id) => {
                self.focused_thread = Some(thread_id);
                self.data = Data::Empty;
                self.request_load();
            }
            RowAction::Decide {
                thread_id,
                request_id,
                choice,
            } => {
                let action = ActionRequest::Decide {
                    thread_id,
                    request_id,
                    choice,
                };
                if self.requires_confirmation(&action) {
                    self.confirmation = Some(action);
                } else {
                    self.dispatch(action);
                }
            }
            RowAction::ResolveWorld {
                thread_id,
                item_id,
                kind,
            } => self.dispatch(ActionRequest::ResolveWorld {
                thread_id,
                item_id,
                kind,
            }),
            RowAction::SyncConnector {
                thread_id,
                connector_id,
            } => self.dispatch(ActionRequest::SyncConnector {
                thread_id,
                connector_id,
            }),
        }
    }

    fn requires_confirmation(&self, action: &ActionRequest) -> bool {
        // 停止执行会收敛在途副作用，不是一次无害的刷新。
        if matches!(action, ActionRequest::AbortThread { .. }) {
            return true;
        }
        let ActionRequest::Decide {
            request_id,
            choice: DecisionChoice::Option(option),
            ..
        } = action
        else {
            return false;
        };
        if option != "approve" {
            return false;
        }
        views::decisions::find_request(&self.data, request_id)
            .is_some_and(|request| request.expected_loss >= ExpectedLoss::High)
    }

    /// 有新的事开始等你时排一条通知。
    ///
    /// 第一次加载不通知：那是数据，不是打扰 —— 你正看着屏幕。
    fn track_new_requests(&mut self, overview: &crate::tui::data::OverviewData) {
        let current: std::collections::BTreeSet<String> =
            views::overview::waiting_requests(overview)
                .into_iter()
                .map(|(_, request)| request.id.to_string())
                .collect();
        let fresh: Vec<String> = current.difference(&self.known_requests).cloned().collect();
        let first_run = self.known_requests.is_empty();
        self.known_requests = current;
        if first_run || fresh.is_empty() {
            return;
        }
        let summary = views::overview::waiting_requests(overview)
            .into_iter()
            .find(|(_, request)| fresh.contains(&request.id.to_string()))
            .map(|(_, request)| request.summary)
            .unwrap_or_else(|| "the runtime needs a decision".into());
        self.notification = Some(summary);
    }

    /// 这条 Thread 现在还能不能被停住。收尾之后就没有"停"这回事了。
    pub fn can_abort(&self) -> bool {
        let Data::ThreadDetail(detail) = &self.data else {
            return false;
        };
        !crate::output::is_terminal(detail.thread.state)
    }

    /// 当前 Thread 上还挂着的第一条请求。
    fn pending_decision(&self) -> Option<(ThreadId, AttentionId)> {
        let Data::ThreadDetail(detail) = &self.data else {
            return self
                .focused_decision
                .as_ref()
                .map(|focus| (focus.thread_id.clone(), focus.request_id.clone()));
        };
        detail
            .thread
            .decisions
            .first()
            .map(|request| (detail.thread.spec.id.clone(), request.id.clone()))
    }

    fn selected_action(&self) -> Option<RowAction> {
        self.rows()
            .get(self.cursor)
            .and_then(|row| row.action.clone())
    }

    /// 选中项只在能做的事之间移动 —— 分组标题和空行不是选项。
    pub fn move_cursor(&mut self, delta: isize) {
        let actionable: Vec<usize> = self
            .rows()
            .iter()
            .enumerate()
            .filter(|(_, row)| row.action.is_some())
            .map(|(index, _)| index)
            .collect();
        if actionable.is_empty() {
            self.cursor = 0;
            return;
        }
        let current = actionable.iter().position(|index| *index == self.cursor);
        let next = match current {
            Some(position) => {
                (position as isize + delta).clamp(0, actionable.len() as isize - 1) as usize
            }
            None if delta > 0 => 0,
            None => actionable.len() - 1,
        };
        self.cursor = actionable[next];
        self.dirty = true;
    }

    fn set_cursor(&mut self, index: usize) {
        self.cursor = index;
        self.clamp_cursor();
    }

    /// 刷新后行会变，光标可能落在一个没有动作的行上；把它拉回最近的可选项。
    fn clamp_cursor(&mut self) {
        let rows = self.rows();
        if rows.is_empty() {
            self.cursor = 0;
            return;
        }
        let start = self.cursor.min(rows.len() - 1);
        if rows[start].action.is_some() {
            self.cursor = start;
            return;
        }
        if let Some(offset) = rows[start..].iter().position(|row| row.action.is_some()) {
            self.cursor = start + offset;
            return;
        }
        self.cursor = rows
            .iter()
            .position(|row| row.action.is_some())
            .unwrap_or(0);
    }
}

/// 上下移动一格，首尾相接。
fn next_index(current: usize, count: usize, delta: isize) -> usize {
    if count == 0 {
        return 0;
    }
    let next = current as isize + delta;
    if next < 0 {
        count - 1
    } else if next as usize >= count {
        0
    } else {
        next as usize
    }
}

pub fn run(mut app: App, events: Receiver<AppEvent>, tui: &mut Tui) -> std::io::Result<()> {
    app.request_load();
    let mut title = String::new();
    while !app.should_quit {
        while let Ok(event) = events.try_recv() {
            app.handle(event);
        }
        app.tick();
        if crossterm::event::poll(Duration::from_millis(50))? {
            match crossterm::event::read()? {
                Event::Key(key) => app.handle_key(key),
                Event::Resize(_, _) => app.handle(AppEvent::Resize),
                Event::Paste(text) => app.handle_paste(&text),
                Event::Mouse(mouse) => match mouse.kind {
                    MouseEventKind::ScrollDown => app.move_cursor(3),
                    MouseEventKind::ScrollUp => app.move_cursor(-3),
                    _ => {}
                },
                _ => {}
            }
        }
        if app.dirty {
            tui.draw(|frame| views::render(frame, &app))?;
            app.dirty = false;
        }
        let wanted = app.window_title();
        if wanted != title {
            tui.set_title(&wanted)?;
            title = wanted;
        }
        if let Some(body) = app.notification.take() {
            tui.notify("else", &body)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::data::{Data, DecisionsData};
    use else_protocol::attention::{AttentionKind, AttentionRequest};
    use else_protocol::ids::{AttentionId, ThreadId};
    use else_protocol::thread::ThreadState;

    fn app() -> (App, Receiver<LoadRequest>, Receiver<ActionRequest>) {
        let (loads, load_rx) = std::sync::mpsc::channel();
        let (actions, action_rx) = std::sync::mpsc::channel();
        (App::new(loads, actions), load_rx, action_rx)
    }

    fn request(id: &str) -> AttentionRequest {
        AttentionRequest {
            id: AttentionId::from(id),
            kind: AttentionKind::Authorization,
            summary: "Approve the release?".into(),
            blocked_reason: "The command changes an external system".into(),
            expected_loss: ExpectedLoss::High,
            blocking: true,
            reversible: false,
            decide_by: None,
            options: vec![else_protocol::attention::DecisionOption {
                id: "approve".into(),
                label: "Approve once".into(),
                recommended: true,
            }],
            default_option: Some("approve".into()),
            related_call_key: Some("call-1".into()),
        }
    }

    fn running_thread(
        state: else_protocol::thread::ThreadState,
    ) -> else_protocol::api::ThreadSnapshot {
        else_protocol::api::ThreadSnapshot {
            spec: else_protocol::thread::ThreadSpec {
                id: ThreadId::from("thread-1"),
                revision: 1,
                outcome: "Prepare the launch deck".into(),
                done_when: Vec::new(),
                constraints: Vec::new(),
                authority: else_protocol::thread::Authority {
                    max_autonomous_effect: else_protocol::effect::SideEffectKind::ReadOnly,
                    forbidden: Vec::new(),
                },
                attention_policy: else_protocol::thread::AttentionPolicy {
                    allow_interrupt: true,
                    batch_interval_minutes: 15,
                },
                mode: Default::default(),
                opaque_harness_policy: Default::default(),
                deadline: None,
                assumptions: Vec::new(),
                project: None,
                origin: None,
                connector_policy: Default::default(),
            },
            state,
            decisions: Vec::new(),
            artifacts: Vec::new(),
            resource_budget: Default::default(),
            resource_usage: Default::default(),
            cursor: 0,
            operation_id: None,
        }
    }

    /// 一条 Thread，挂着给定的待决请求。
    fn overview_with(requests: Vec<(&str, &str)>) -> Data {
        let mut thread = running_thread(else_protocol::thread::ThreadState::NeedsDecision);
        thread.decisions = requests
            .into_iter()
            .map(|(id, summary)| AttentionRequest {
                summary: summary.into(),
                ..request(id)
            })
            .collect();
        Data::Overview(Box::new(crate::tui::data::OverviewData {
            brief: else_protocol::world::PersonalBrief {
                generated_at: chrono::Utc::now(),
                open_commitments: Vec::new(),
                waiting_external: Vec::new(),
                overdue: Vec::new(),
                action_items_due_soon: Vec::new(),
                risks: Vec::new(),
                upcoming_calendar: Vec::new(),
            },
            attention: Vec::new(),
            threads: vec![thread],
        }))
    }

    fn runtime_event() -> else_protocol::api::EventResponse {
        else_protocol::api::EventResponse {
            seq: 7,
            event_id: "e-7".into(),
            thread_id: "i-1".into(),
            scope_id: "s-1".into(),
            run_id: None,
            generation: 1,
            thread_revision: 1,
            occurred_at: "2026-10-31T00:00:00Z".into(),
            payload: serde_json::Value::Null,
        }
    }

    #[test]
    fn stale_response_releases_the_worker_without_replacing_the_new_screen() {
        let (mut app, _loads, _actions) = app();
        app.request_load();
        let stale = app.generation;
        app.goto(Screen::Results);
        app.handle(AppEvent::Loaded {
            generation: stale,
            data: Data::Threads(Vec::new()),
        });
        assert!(app.inflight.is_none());
        assert!(app.refresh_again);
        assert!(matches!(app.data, Data::Empty));
    }

    #[test]
    fn runtime_event_during_load_is_not_lost() {
        let (mut app, _loads, _actions) = app();
        app.goto(Screen::Results);
        let generation = app.generation;
        app.handle(AppEvent::RuntimeEvent(runtime_event()));
        app.handle(AppEvent::Loaded {
            generation,
            data: Data::Threads(Vec::new()),
        });
        assert!(app.refresh_again);
    }

    #[test]
    fn opening_a_decision_does_not_execute_it() {
        let (mut app, _loads, actions) = app();
        app.screen = Screen::Decisions;
        app.data = Data::Decisions(Box::new(DecisionsData {
            windows: Vec::new(),
            loose: vec![(ThreadId::from("thread-1"), request("request-1"))],
        }));
        app.cursor = app
            .rows()
            .iter()
            .position(|row| matches!(row.action, Some(RowAction::OpenDecision { .. })))
            .unwrap();
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(app.screen, Screen::DecisionDetail);
        assert!(!app.action_pending);
        assert!(actions.try_recv().is_err());
    }

    #[test]
    fn loading_selects_the_first_action_and_refresh_preserves_its_identity() {
        let (mut app, _loads, _actions) = app();
        app.goto(Screen::Decisions);
        let generation = app.generation;
        app.handle(AppEvent::Loaded {
            generation,
            data: Data::Decisions(Box::new(DecisionsData {
                windows: Vec::new(),
                loose: vec![(ThreadId::from("thread-1"), request("request-1"))],
            })),
        });
        assert!(matches!(
            app.selected_action(),
            Some(RowAction::OpenDecision { request_id, .. }) if request_id == AttentionId::from("request-1")
        ));

        app.request_load();
        app.handle(AppEvent::Loaded {
            generation: app.generation,
            data: Data::Decisions(Box::new(DecisionsData {
                windows: Vec::new(),
                loose: vec![(ThreadId::from("thread-1"), request("request-1"))],
            })),
        });
        assert!(matches!(
            app.selected_action(),
            Some(RowAction::OpenDecision { request_id, .. }) if request_id == AttentionId::from("request-1")
        ));
    }

    #[test]
    fn non_approval_rejection_is_sent_without_confirmation() {
        let (mut app, _loads, actions) = app();
        app.screen = Screen::DecisionDetail;
        app.focused_decision = Some(DecisionFocus {
            thread_id: ThreadId::from("thread-1"),
            request_id: AttentionId::from("request-1"),
        });
        let mut request = request("request-1");
        request.options = vec![else_protocol::attention::DecisionOption {
            id: "reject".into(),
            label: "Reject".into(),
            recommended: true,
        }];
        app.data = Data::Decisions(Box::new(DecisionsData {
            windows: Vec::new(),
            loose: vec![(ThreadId::from("thread-1"), request)],
        }));
        app.cursor = app
            .rows()
            .iter()
            .position(|row| matches!(row.action, Some(RowAction::Decide { .. })))
            .unwrap();
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(app.confirmation.is_none());
        assert!(matches!(
            actions.try_recv(),
            Ok(ActionRequest::Decide {
                choice: DecisionChoice::Option(option),
                ..
            }) if option == "reject"
        ));
    }

    #[test]
    fn composer_paste_is_normalized_and_custom_decision_is_submitted() {
        let (mut app, _loads, actions) = app();
        app.screen = Screen::DecisionDetail;
        app.focused_decision = Some(DecisionFocus {
            thread_id: ThreadId::from("thread-1"),
            request_id: AttentionId::from("request-1"),
        });
        app.handle_key(KeyEvent::new(KeyCode::Char('e'), KeyModifiers::NONE));
        assert_eq!(app.screen, Screen::Composer);
        app.handle_paste("  first line\r\nsecond   line  ");
        assert_eq!(app.composer.text(), "first line second line");
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(app.screen, Screen::DecisionDetail);
        assert!(matches!(
            actions.try_recv(),
            Ok(ActionRequest::Decide {
                choice: DecisionChoice::FreeText(text),
                ..
            }) if text == "first line second line"
        ));
    }

    #[test]
    fn high_impact_approval_requires_confirmation() {
        let (mut app, _loads, actions) = app();
        app.screen = Screen::DecisionDetail;
        app.focused_decision = Some(DecisionFocus {
            thread_id: ThreadId::from("thread-1"),
            request_id: AttentionId::from("request-1"),
        });
        app.data = Data::Decisions(Box::new(DecisionsData {
            windows: Vec::new(),
            loose: vec![(ThreadId::from("thread-1"), request("request-1"))],
        }));
        app.cursor = app
            .rows()
            .iter()
            .position(|row| matches!(row.action, Some(RowAction::Decide { .. })))
            .unwrap();
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(app.confirmation.is_some());
        assert!(!app.action_pending);
        assert!(actions.try_recv().is_err());
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(app.action_pending);
        assert!(actions.try_recv().is_ok());
    }

    #[test]
    fn selection_only_lands_on_rows_that_do_something() {
        let (mut app, _loads, _actions) = app();
        app.screen = Screen::Decisions;
        app.data = Data::Decisions(Box::new(DecisionsData {
            windows: Vec::new(),
            loose: vec![
                (ThreadId::from("thread-1"), request("request-1")),
                (ThreadId::from("thread-2"), request("request-2")),
            ],
        }));
        app.set_cursor(0);
        // 第 0 行是分组标题：它不是一个能选的东西。
        assert!(app.selected_action().is_some());
        app.move_cursor(1);
        assert!(matches!(
            app.selected_action(),
            Some(RowAction::OpenDecision { request_id, .. })
                if request_id == AttentionId::from("request-2")
        ));
        app.move_cursor(10);
        assert!(
            matches!(
                app.selected_action(),
                Some(RowAction::OpenDecision { request_id, .. })
                    if request_id == AttentionId::from("request-2")
            ),
            "the cursor stops at the last actionable row"
        );
    }

    #[test]
    fn stopping_a_running_thread_asks_once_before_it_stops() {
        let (mut app, _loads, actions) = app();
        app.screen = Screen::ThreadDetail;
        app.focused_thread = Some(ThreadId::from("thread-1"));
        app.data = Data::ThreadDetail(Box::new(crate::tui::data::ThreadDetailData {
            thread: running_thread(ThreadState::Active),
            scopes: Vec::new(),
            plan: else_protocol::api::PlanView {
                current: None,
                history: Vec::new(),
                impacts: Vec::new(),
            },
            resources: else_protocol::api::ResourceView {
                budget: Default::default(),
                usage: Default::default(),
                sandboxes: Vec::new(),
            },
            events: Vec::new(),
        }));

        app.handle_key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE));
        // 停止会收敛在途副作用：先问一次，别一键就不可逆。
        assert!(app.confirmation.is_some());
        assert!(actions.try_recv().is_err());

        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(matches!(
            actions.try_recv().expect("abort dispatched"),
            ActionRequest::AbortThread { .. }
        ));
    }

    #[test]
    fn a_finished_thread_cannot_be_stopped() {
        let (mut app, _loads, actions) = app();
        app.screen = Screen::ThreadDetail;
        app.focused_thread = Some(ThreadId::from("thread-1"));
        app.data = Data::ThreadDetail(Box::new(crate::tui::data::ThreadDetailData {
            thread: running_thread(ThreadState::Cancelled),
            scopes: Vec::new(),
            plan: else_protocol::api::PlanView {
                current: None,
                history: Vec::new(),
                impacts: Vec::new(),
            },
            resources: else_protocol::api::ResourceView {
                budget: Default::default(),
                usage: Default::default(),
                sandboxes: Vec::new(),
            },
            events: Vec::new(),
        }));
        assert!(!app.can_abort());
        app.handle_key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE));
        assert!(app.confirmation.is_none());
        assert!(actions.try_recv().is_err());
    }

    #[test]
    fn the_command_palette_says_slash_and_offers_what_works_here() {
        let (mut app, _loads, _actions) = app();
        app.handle_key(KeyEvent::new(KeyCode::Char('/'), KeyModifiers::NONE));
        assert!(app.palette.is_some());
        let commands = app.palette.as_ref().unwrap().filtered(&app);
        let titles: Vec<&str> = commands.iter().map(|command| command.title()).collect();
        assert!(titles.contains(&"Review decisions"), "{titles:?}");
        // 首屏没有"停这条运行"可言 —— 面板承诺不了它做不到的事。
        assert!(!titles.contains(&"Stop this run"), "{titles:?}");
    }

    #[test]
    fn the_command_palette_filters_as_you_type() {
        let (mut app, _loads, _actions) = app();
        app.handle_key(KeyEvent::new(KeyCode::Char('/'), KeyModifiers::NONE));
        for character in "every".chars() {
            app.handle_key(KeyEvent::new(KeyCode::Char(character), KeyModifiers::NONE));
        }
        let commands = app.palette.as_ref().unwrap().filtered(&app);
        assert_eq!(commands.len(), 1, "{commands:?}");
        assert_eq!(commands[0].title(), "See every thread");
    }

    #[test]
    fn only_new_requests_are_announced_outside_the_window() {
        let (mut app, _loads, _actions) = app();
        app.screen = Screen::Overview;
        let first = overview_with(vec![("request-1", "Approve the release")]);
        app.handle(AppEvent::Loaded {
            generation: app.generation,
            data: first.clone(),
        });
        // 第一次加载是数据，不是打扰。
        assert!(app.notification.is_none());

        app.request_load();
        app.handle(AppEvent::Loaded {
            generation: app.generation,
            data: first,
        });
        assert!(app.notification.is_none(), "nothing new happened");

        app.request_load();
        app.handle(AppEvent::Loaded {
            generation: app.generation,
            data: overview_with(vec![
                ("request-1", "Approve the release"),
                ("request-2", "Overwrite outline.md"),
            ]),
        });
        assert_eq!(app.notification.as_deref(), Some("Overwrite outline.md"));
    }

    #[test]
    fn any_key_closes_help() {
        let (mut app, _loads, _actions) = app();
        app.show_help = true;
        app.handle_key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE));
        assert!(!app.show_help);
        assert!(!app.should_quit);
    }
}
