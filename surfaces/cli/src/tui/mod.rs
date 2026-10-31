//! 终端界面。
//!
//! 架构抄自 Codex CLI（`codex-rs/tui`）最要紧的两点：
//!
//! 1. **组件从不直接调后端**。界面只发 [`app_event::AppEvent`]，
//!    顶层 `App` 统一处理并发起来源；异步结果走同一条消息总线回来。
//! 2. **界面与数据之间只有一条通路**。界面读到的永远是 Runtime 真正返回的
//!    东西，不会出现「屏幕上是 A、账本里是 B」。
//!
//! 但界面**不是**聊天框：else 管的是意图、世界与注意力，
//! 所以首页是 Daily Brief，不是对话。

pub mod app;
pub mod app_event;
pub mod composer;
pub mod data;
pub mod loader;
pub mod row;
pub mod terminal;
pub mod theme;
pub mod views;

use std::sync::mpsc;

use else_client::RuntimeClient;

use crate::error::Result;

/// 启动终端界面。
pub fn launch(client: RuntimeClient) -> Result<()> {
    let (events, receiver) = mpsc::channel();
    let (work, worker) = loader::spawn_worker(client.clone(), events.clone());
    let _stream = loader::spawn_event_stream(client.clone(), events.clone());
    // 后台线程的返回值不重要：它们自己会结束。
    drop(worker);
    let mut tui = terminal::Tui::enter().map_err(|error| {
        crate::error::CliError::usage(format!("cannot enter terminal: {error}"))
    })?;
    let application = app::App::new(work);
    app::run(application, receiver, &mut tui)
        .map_err(|error| crate::error::CliError::usage(format!("terminal error: {error}")))
}
