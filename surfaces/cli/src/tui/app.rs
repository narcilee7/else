//! 顶层应用：状态、事件循环与按键分发。
//!
//! 借鉴 Codex CLI 的结构：界面组件**从不直接调后端**，只发事件；
//! 顶层 App 统一处理并发起来源。这里同样只有一条数据通路：
//! 界面读到的永远是 Runtime 真正返回的东西。

use std::sync::mpsc::{Receiver, Sender};
use std::time::Duration;

use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use else_protocol::attention::DecisionChoice;
use else_protocol::ids::IntentId;

use super::app_event::AppEvent;
use super::composer::Composer;
use super::data::Data;
use super::loader::{ActionRequest, LoadRequest, Work};
use super::row::{Row, RowAction};
use super::terminal::Tui;
use super::views;

/// 当前屏幕。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    /// 首页：今天真正要你管的事。
    Overview,
    /// Intent 列表。
    Intents,
    /// Intent 详情。
    IntentDetail,
    /// World 条目。
    World,
    /// 待决策队列。
    Decisions,
    /// Connector 列表。
    Connectors,
    /// 新建 Intent 的输入框。
    Composer,
}

impl Screen {
    /// 屏幕标题。
    pub fn title(&self) -> &'static str {
        match self {
            Self::Overview => "Overview",
            Self::Intents => "Intents",
            Self::IntentDetail => "Intent",
            Self::World => "World",
            Self::Decisions => "Decisions",
            Self::Connectors => "Connectors",
            Self::Composer => "New intent",
        }
    }
}

/// 底部状态条。
#[derive(Debug, Clone)]
pub struct Status {
    pub text: String,
    pub failed: bool,
}

/// 应用状态。
pub struct App {
    pub screen: Screen,
    pub data: Data,
    /// 当前选中的行。
    pub cursor: usize,
    /// 列表滚动偏移。
    pub scroll: u16,
    pub status: Option<Status>,
    pub composer: Composer,
    /// 加载代次：用来丢弃过期响应。
    pub generation: u64,
    /// 正在飞行的加载请求的代次；`None` 表示当前没有请求在飞。
    inflight: Option<u64>,
    /// 有新动静，到了再刷新。
    refresh_again: bool,
    /// 上次发起刷新的时刻：用来限流。
    last_refresh: std::time::Instant,
    pub dirty: bool,
    pub should_quit: bool,
    pub show_help: bool,
    /// 当前聚焦的 Intent；World 与 Connector 屏幕按它取数。
    pub focused_intent: Option<IntentId>,
    work: Sender<Work>,
}

impl App {
    /// 构造应用。
    pub fn new(work: Sender<Work>) -> Self {
        Self {
            screen: Screen::Overview,
            data: Data::Empty,
            cursor: 0,
            scroll: 0,
            status: None,
            composer: Composer::new(),
            generation: 0,
            inflight: None,
            refresh_again: false,
            last_refresh: std::time::Instant::now(),
            dirty: true,
            should_quit: false,
            show_help: false,
            focused_intent: None,
            work,
        }
    }

    /// 当前屏幕的行（所有面板拍平）。
    ///
    /// 光标在这个拍平后的序列上移动；渲染时再按面板切回去。
    pub fn rows(&self) -> Vec<Row> {
        views::flat_rows(self)
    }

    /// 发起一次加载。
    ///
    /// 同一时刻只允许一个请求在飞：后来的请求不能作废前一个的结果 ——
    /// 否则事件流一密集，界面就永远停在「加载中」。
    pub fn request_load(&mut self) {
        if self.inflight.is_some() {
            self.refresh_again = true;
            return;
        }
        self.generation += 1;
        let generation = self.generation;
        let request = match self.screen {
            Screen::Overview => LoadRequest::Overview { generation },
            Screen::Intents => LoadRequest::Intents { generation },
            Screen::IntentDetail => match self.resolve_intent() {
                Some(intent_id) => LoadRequest::IntentDetail {
                    generation,
                    intent_id,
                },
                None => LoadRequest::Intents { generation },
            },
            Screen::World => match self.resolve_intent() {
                Some(intent_id) => LoadRequest::World {
                    generation,
                    intent_id,
                },
                None => LoadRequest::Intents { generation },
            },
            Screen::Decisions => LoadRequest::Decisions { generation },
            Screen::Connectors => match self.resolve_intent() {
                Some(intent_id) => LoadRequest::Connectors {
                    generation,
                    intent_id,
                },
                None => LoadRequest::Intents { generation },
            },
            Screen::Composer => LoadRequest::Overview { generation },
        };
        self.inflight = Some(generation);
        self.last_refresh = std::time::Instant::now();
        let _ = self.work.send(Work::Load(request));
    }

    /// 心跳：把积压的刷新请求发出去。
    ///
    /// 限流到每秒一次：Runtime 事件可能连珠炮似地来，
    /// 但界面没必要为此反复重取同一份数据。
    pub fn tick(&mut self) {
        if !self.refresh_again || self.inflight.is_some() {
            return;
        }
        if self.last_refresh.elapsed() < std::time::Duration::from_millis(1000) {
            return;
        }
        self.refresh_again = false;
        self.request_load();
    }

    /// 当前作用的 Intent。
    ///
    /// 没聚焦过就用最近一个 Intent —— World 与 Connector 是按 Intent 组织的，
    /// 用户不该为了看一眼世界先手动选一个。
    fn resolve_intent(&mut self) -> Option<IntentId> {
        if let Some(intent_id) = self.focused_intent.clone() {
            return Some(intent_id);
        }
        let candidate = match &self.data {
            Data::Overview(data) => data.intents.first().map(|intent| intent.spec.id.clone()),
            Data::Intents(intents) => intents.first().map(|intent| intent.spec.id.clone()),
            _ => None,
        };
        self.focused_intent = candidate.clone();
        candidate
    }

    /// 切屏幕。
    pub fn goto(&mut self, screen: Screen) {
        if self.screen == screen {
            return;
        }
        self.screen = screen;
        self.cursor = 0;
        self.scroll = 0;
        self.dirty = true;
        self.request_load();
    }

    /// 处理一个内部事件。
    pub fn handle(&mut self, event: AppEvent) {
        match event {
            AppEvent::Resize => self.dirty = true,
            AppEvent::Loaded { generation, data } => {
                // 过期响应直接丢掉：用户连按几下切换屏幕时，
                // 先发起的请求可能后到，迟到的结果不能盖掉新的。
                if self.inflight != Some(generation) {
                    return;
                }
                self.inflight = None;
                self.data = data;
                self.dirty = true;
                self.refresh_again = false;
            }
            AppEvent::LoadFailed { generation, error } => {
                if self.inflight != Some(generation) {
                    return;
                }
                self.inflight = None;
                self.status = Some(Status {
                    text: error,
                    failed: true,
                });
                self.dirty = true;
                self.refresh_again = false;
            }
            AppEvent::RuntimeEvent(event) => {
                // Runtime 有新动静：记下最新的一条，让用户知道「刚才那件事真的动了」，
                // 并安排一次刷新。
                //
                // 注意这里**不**直接发起加载：事件可能连珠炮似地来，
                // 每次都重新加载会把正在飞行的请求作废，界面就永远停在「加载中」。
                self.status = Some(Status {
                    text: format!("event #{} {}", event.seq, event.scope_id),
                    failed: false,
                });
                self.refresh_again = true;
                self.dirty = true;
            }
            AppEvent::ActionStarted => {
                self.status = Some(Status {
                    text: "working…".into(),
                    failed: false,
                });
                self.dirty = true;
            }
            AppEvent::ActionDone {
                label,
                detail,
                intent_id,
            } => {
                self.status = Some(Status {
                    text: format!("{label}: {detail}"),
                    failed: false,
                });
                if let Some(intent_id) = intent_id {
                    self.focused_intent = Some(intent_id);
                }
                self.refresh_again = true;
            }
            AppEvent::ActionFailed { error } => {
                self.status = Some(Status {
                    text: error,
                    failed: true,
                });
                self.dirty = true;
            }
        }
    }

    /// 处理按键。
    pub fn handle_key(&mut self, key: KeyEvent) {
        if key.kind != KeyEventKind::Press {
            return;
        }
        // Ctrl-C 永远是退出。
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            self.should_quit = true;
            return;
        }
        if self.show_help {
            match key.code {
                KeyCode::Esc | KeyCode::Char('?') | KeyCode::Enter | KeyCode::Char('q') => {
                    self.show_help = false;
                    self.dirty = true;
                }
                _ => {}
            }
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
            KeyCode::Char('2') => self.goto(Screen::Intents),
            KeyCode::Char('3') => self.goto(Screen::World),
            KeyCode::Char('4') => self.goto(Screen::Decisions),
            KeyCode::Char('5') => self.goto(Screen::Connectors),
            KeyCode::Char('r') => self.request_load(),
            KeyCode::Char('n') => {
                self.composer.clear();
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
            KeyCode::Esc => self.focused_intent = None,
            _ => self.screen_key(key),
        }
        self.dirty = true;
    }

    /// 屏幕特有的按键。
    fn screen_key(&mut self, key: KeyEvent) {
        match self.screen {
            Screen::Decisions => {
                let choice = match key.code {
                    KeyCode::Char('a') => DecisionChoice::Option("approve".into()),
                    KeyCode::Char('r') => DecisionChoice::Option("reject".into()),
                    KeyCode::Char('s') => DecisionChoice::Option("defer".into()),
                    _ => return,
                };
                if let Some(RowAction::Decide {
                    intent_id,
                    request_id,
                    ..
                }) = self.selected_action()
                {
                    self.dispatch(ActionRequest::Decide {
                        intent_id,
                        request_id,
                        choice,
                    });
                }
            }
            Screen::World => {
                let kind = match key.code {
                    KeyCode::Char('c') => else_protocol::world::WorldItemResolutionKind::Completed,
                    KeyCode::Char('d') => else_protocol::world::WorldItemResolutionKind::Dismissed,
                    _ => return,
                };
                if let Some(RowAction::ResolveWorld {
                    intent_id, item_id, ..
                }) = self.selected_action()
                {
                    self.dispatch(ActionRequest::ResolveWorld {
                        intent_id,
                        item_id,
                        kind,
                    });
                }
            }
            Screen::Connectors if key.code == KeyCode::Char('s') => {
                if let Some(RowAction::SyncConnector {
                    intent_id,
                    connector_id,
                }) = self.selected_action()
                {
                    self.dispatch(ActionRequest::SyncConnector {
                        intent_id,
                        connector_id,
                    });
                }
            }
            Screen::IntentDetail if key.code == KeyCode::Char('a') => {
                if let Some(intent_id) = self.focused_intent.clone() {
                    self.dispatch(ActionRequest::AdvanceIntent { intent_id });
                }
            }
            _ => {}
        }
    }

    /// 输入框里的按键。
    fn handle_composer_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => {
                self.goto(Screen::Overview);
                return;
            }
            KeyCode::Enter => {
                let outcome = self.composer.text().trim().to_string();
                if !outcome.is_empty() {
                    self.dispatch(ActionRequest::CreateIntent { outcome });
                    self.goto(Screen::Overview);
                } else {
                    self.goto(Screen::Overview);
                }
                return;
            }
            _ => {}
        }
        if self.composer.handle(key) {
            self.dirty = true;
        }
    }

    /// 把一个动作丢给后台线程。
    fn dispatch(&mut self, action: ActionRequest) {
        let _ = self.work.send(Work::Action(action));
    }

    /// 触发当前选中行的动作。
    fn activate(&mut self) {
        let Some(action) = self.selected_action() else {
            return;
        };
        match action {
            RowAction::OpenIntent(intent_id) => {
                self.focused_intent = Some(intent_id);
                self.goto(Screen::IntentDetail);
            }
            RowAction::AdvanceIntent(intent_id) => {
                self.dispatch(ActionRequest::AdvanceIntent { intent_id });
            }
            RowAction::Decide {
                intent_id,
                request_id,
                choice,
            } => {
                self.dispatch(ActionRequest::Decide {
                    intent_id,
                    request_id,
                    choice,
                });
            }
            RowAction::ResolveWorld {
                intent_id,
                item_id,
                kind,
            } => {
                self.dispatch(ActionRequest::ResolveWorld {
                    intent_id,
                    item_id,
                    kind,
                });
            }
            RowAction::SyncConnector {
                intent_id,
                connector_id,
            } => {
                self.dispatch(ActionRequest::SyncConnector {
                    intent_id,
                    connector_id,
                });
            }
        }
    }

    /// 当前选中行的动作。
    fn selected_action(&self) -> Option<RowAction> {
        self.rows()
            .get(self.cursor)
            .and_then(|row| row.action.clone())
    }

    fn move_cursor(&mut self, delta: isize) {
        let count = self.rows().len();
        if count == 0 {
            self.cursor = 0;
            return;
        }
        let next = self.cursor as isize + delta;
        self.cursor = next.clamp(0, count as isize - 1) as usize;
    }

    fn set_cursor(&mut self, index: usize) {
        let count = self.rows().len();
        if count == 0 {
            self.cursor = 0;
            return;
        }
        self.cursor = index.min(count - 1);
    }
}

/// 主循环。
pub fn run(mut app: App, events: Receiver<AppEvent>, tui: &mut Tui) -> std::io::Result<()> {
    app.request_load();
    while !app.should_quit {
        // 后台送来的事件：先处理完，再看输入。
        while let Ok(event) = events.try_recv() {
            app.handle(event);
        }
        // 心跳：把积压的刷新发出去（限流在 App::tick 里）。
        app.tick();
        // 输入：最多等 50ms，这样后台事件不会被饿死。
        if crossterm::event::poll(Duration::from_millis(50))? {
            match crossterm::event::read()? {
                Event::Key(key) => app.handle_key(key),
                Event::Resize(_, _) => app.handle(AppEvent::Resize),
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
    use crate::tui::data::Data;

    fn app() -> App {
        let (work, _receiver) = std::sync::mpsc::channel();
        // 这里只测状态机，不碰网络。
        App::new(work)
    }

    #[test]
    fn a_second_request_does_not_invalidate_the_first_response() {
        let mut app = app();
        app.request_load();
        let first = app.generation;
        // 事件流又来了一下：不能把在飞行的请求作废，
        // 否则界面永远停在「加载中」。
        app.request_load();
        assert_eq!(app.inflight, Some(first));

        app.handle(AppEvent::Loaded {
            generation: first,
            data: Data::Intents(Vec::new()),
        });
        assert!(app.inflight.is_none());
        assert!(matches!(app.data, Data::Intents(_)));
    }

    #[test]
    fn a_stale_response_is_dropped() {
        let mut app = app();
        app.request_load();
        let stale = app.generation;
        app.handle(AppEvent::Loaded {
            generation: stale,
            data: Data::Intents(Vec::new()),
        });
        app.request_load();
        let current = app.generation;
        assert_ne!(stale, current);
        // 迟到的旧响应不能盖掉新的。
        app.handle(AppEvent::Loaded {
            generation: stale,
            data: Data::Decisions(Box::new(crate::tui::data::DecisionsData {
                windows: Vec::new(),
                loose: Vec::new(),
            })),
        });
        assert_eq!(app.inflight, Some(current));
        assert!(matches!(app.data, Data::Intents(_)));
    }

    #[test]
    fn runtime_events_do_not_start_a_new_load_immediately() {
        let mut app = app();
        app.request_load();
        let generation = app.generation;
        app.handle(AppEvent::RuntimeEvent(else_protocol::api::EventResponse {
            seq: 7,
            event_id: "e-7".into(),
            intent_id: "i-1".into(),
            scope_id: "s-1".into(),
            run_id: None,
            generation: 1,
            intent_revision: 1,
            occurred_at: "2026-10-31T00:00:00Z".into(),
            payload: serde_json::Value::Null,
        }));
        // 事件只标记「该刷新了」，不重发请求 ——
        // 否则连珠炮似的事件会把每次响应都作废。
        assert_eq!(app.inflight, Some(generation));
        assert!(app.refresh_again);
    }
}
