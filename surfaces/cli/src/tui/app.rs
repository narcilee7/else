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

impl Screen {
    pub fn title(&self) -> &'static str {
        match self {
            Self::Overview => "Today",
            Self::Decisions => "Decisions",
            Self::Results => "Results",
            Self::ThreadDetail => "Thread",
            Self::DecisionDetail => "Decision",
            Self::Connectors => "Settings",
            Self::Composer => "New thread",
        }
    }
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
                if self.screen == Screen::DecisionDetail {
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
            KeyCode::Char('1') => self.goto(Screen::Overview),
            KeyCode::Char('2') => self.goto(Screen::Decisions),
            KeyCode::Char('3') => self.goto(Screen::Results),
            KeyCode::Char('4') => self.goto(Screen::Connectors),
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
            RowAction::AdvanceThread(thread_id) => {
                self.dispatch(ActionRequest::AdvanceThread { thread_id });
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

    fn selected_action(&self) -> Option<RowAction> {
        self.rows()
            .get(self.cursor)
            .and_then(|row| row.action.clone())
    }

    pub fn move_cursor(&mut self, delta: isize) {
        let count = self.rows().len();
        if count == 0 {
            self.cursor = 0;
            return;
        }
        let next = self.cursor as isize + delta;
        self.cursor = next.clamp(0, count as isize - 1) as usize;
        self.dirty = true;
    }

    fn set_cursor(&mut self, index: usize) {
        self.cursor = index;
        self.clamp_cursor();
    }

    fn clamp_cursor(&mut self) {
        let count = self.rows().len();
        self.cursor = if count == 0 {
            0
        } else {
            self.cursor.min(count - 1)
        };
    }
}

pub fn run(mut app: App, events: Receiver<AppEvent>, tui: &mut Tui) -> std::io::Result<()> {
    app.request_load();
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
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::data::{Data, DecisionsData};
    use else_protocol::attention::{AttentionKind, AttentionRequest};
    use else_protocol::ids::{AttentionId, ThreadId};

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
    fn any_key_closes_help() {
        let (mut app, _loads, _actions) = app();
        app.show_help = true;
        app.handle_key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE));
        assert!(!app.show_help);
        assert!(!app.should_quit);
    }
}
