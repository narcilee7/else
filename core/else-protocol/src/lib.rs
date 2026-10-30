//! else 核心协议。
//!
//! 协议是第一公民：实现可以重写，协议语义必须稳定、可版本化。
//! 所有类型可序列化为 JSON，事件 schema 只加不删，破坏性变更升级 `PROTOCOL_VERSION`。

pub mod artifact;
pub mod attention;
pub mod effect;
pub mod event;
pub mod harness;
pub mod ids;
pub mod intent;
pub mod memory;
pub mod resource;

/// 协议版本。事件写入时记录，重放时校验兼容性。
pub const PROTOCOL_VERSION: &str = "0.1.0";
