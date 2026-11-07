//! 终端界面。
//!
//! 组件不直接访问后端；输入、加载、动作与 Runtime 事件都由顶层 `App`
//! 统一协调。首屏回答的是「现在什么在等我」，不是聊天窗口或调试控制台。

pub mod app;
pub mod app_event;
pub mod composer;
pub mod data;
pub mod loader;
pub mod palette;
pub mod row;
pub mod terminal;
pub mod theme;
pub mod views;

use std::sync::mpsc;

use crate::error::Result;
use crate::session::Session;

/// 启动终端界面。
///
/// `session` 必须一直活到界面退出：没有常驻 daemon 时，它内部持有内嵌
/// daemon 的生命周期 guard。只把 client 移出来会让后端在首帧之前停止。
pub fn launch(session: Session) -> Result<()> {
    let client = session.client().clone();
    let (events, receiver) = mpsc::channel();
    let (loads, load_worker) = loader::spawn_load_worker(client.clone(), events.clone());
    let (actions, action_worker) = loader::spawn_action_worker(client.clone(), events.clone());
    let _stream = loader::spawn_event_stream(client, events.clone());
    let mut tui = terminal::Tui::enter().map_err(|error| {
        crate::error::CliError::usage(format!("cannot enter terminal: {error}"))
    })?;
    let mut application = app::App::new(loads, actions);
    if session.embedded() {
        application.runtime_note = Some("runtime 临时：后台没有常驻服务".into());
    }
    let result = app::run(application, receiver, &mut tui)
        .map_err(|error| crate::error::CliError::usage(format!("terminal error: {error}")));
    drop(tui);
    let _ = load_worker.join();
    let _ = action_worker.join();
    result
}
