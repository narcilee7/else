//! 内部消息总线。
//!
//! 界面组件只发事件，顶层 `App` 统一处理输入、后台加载和动作结果。

use else_protocol::api::EventResponse;

use super::data::Data;

/// 应用内部事件。
#[derive(Debug)]
pub enum AppEvent {
    /// 终端尺寸变化。
    Resize,
    /// 数据加载完成。
    Loaded { generation: u64, data: Data },
    /// 加载失败。
    LoadFailed { generation: u64, error: String },
    /// Runtime 推送的新事件。
    RuntimeEvent(EventResponse),
    /// 一次用户动作的结果。
    ActionDone {
        label: String,
        detail: String,
        thread_id: Option<else_protocol::ids::ThreadId>,
    },
    /// 一次用户动作失败。
    ActionFailed { error: String },
}
