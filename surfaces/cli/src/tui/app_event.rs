//! 内部消息总线。
//!
//! 这是从 Codex CLI 抄来的最要紧的一个模式：**组件从不直接调后端**。
//! 组件只发事件，顶层 `App` 统一处理并发起来源。异步结果也走同一条路回来。
//!
//! 于是「界面」和「数据」之间只有一条通道：没有第二个真相来源，
//! 也不会出现「界面显示了 A，账本里其实是 B」的情况。

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
    /// 一次用户动作开始执行。
    ActionStarted,
    /// 一次用户动作的结果。
    ActionDone {
        label: String,
        detail: String,
        /// 动作涉及的 Intent；界面据此决定要不要跳过去。
        intent_id: Option<else_protocol::ids::IntentId>,
    },
    /// 一次用户动作失败。
    ActionFailed { error: String },
}
