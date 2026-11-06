//! else Runtime 客户端：所有 surface 共用的 HTTP 传输层。
//!
//! CLI、TUI 与 macOS App 走同一套契约：同一份 [`else_protocol::api`] 形状、
//! 同一个 Bearer 令牌、同一套 Revision 前置条件。这样「终端看到的」和
//! 「App 看到的」永远是同一个世界。
//!
//! 设计取舍：
//!
//! - **只有一个后端**。客户端永远说 HTTP；需要本地运行时，调用方负责起一个
//!   daemon 再连它。这样不存在「进程内一套语义、HTTP 一套语义」的漂移。
//! - **Revision 前置条件由客户端维护**。客户端缓存每个 Thread 的 Revision，
//!   写操作时用 `If-Match` 带上它。并发修改会被 daemon 以 409 拒绝，
//!   而不是被静默覆盖。

pub mod config;
pub mod http;
pub mod stream;

pub use config::{default_runtime_dir, ClientConfig, DEFAULT_RUNTIME_URL};
pub use http::{RuntimeClient, RuntimeOperation, RuntimeVersion};
pub use stream::EventStream;

/// 客户端错误。
#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    /// Runtime 不可达：没启动、地址不对、端口被占。
    #[error("runtime unreachable: {0}")]
    Unreachable(String),
    /// Runtime 明确拒绝了这个请求。
    #[error("runtime rejected the request: {0}")]
    Rejected(String),
    /// 响应不符合契约。
    #[error("malformed runtime response: {0}")]
    Malformed(String),
    /// 缺少访问令牌。
    #[error("missing runtime token: {0}")]
    MissingToken(String),
}

impl ClientError {
    /// 缺省构造一个缺令牌的错误。
    pub fn missing_token() -> Self {
        Self::MissingToken(
            "no token in ELSE_RUNTIME_TOKEN and no readable daemon-token file".into(),
        )
    }
}

pub type Result<T> = std::result::Result<T, ClientError>;
