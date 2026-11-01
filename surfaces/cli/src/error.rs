//! CLI 错误类型。
//!
//! CLI 只做参数解析与呈现：它可能撞上 Runtime 拒绝、HTTP 传输失败，
//! 或者用户参数不对。三类错误分开，提示才能说清「是谁的问题」。

use std::fmt;

/// CLI 错误。
#[derive(Debug)]
pub enum CliError {
    /// Runtime 拒绝了这次操作。
    Api(else_api::ApiError),
    /// Runtime 不可达或响应不符合契约。
    Client(else_client::ClientError),
    /// 用户参数不对。
    Usage(String),
}

impl fmt::Display for CliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Api(error) => write!(f, "{error}"),
            Self::Client(error) => write!(f, "{error}"),
            Self::Usage(message) => write!(f, "{message}"),
        }
    }
}

impl std::error::Error for CliError {}

impl From<else_api::ApiError> for CliError {
    fn from(error: else_api::ApiError) -> Self {
        Self::Api(error)
    }
}

impl From<else_client::ClientError> for CliError {
    fn from(error: else_client::ClientError) -> Self {
        Self::Client(error)
    }
}

impl From<std::io::Error> for CliError {
    fn from(error: std::io::Error) -> Self {
        Self::Usage(error.to_string())
    }
}

impl CliError {
    /// 构造一个用法错误。
    pub fn usage(message: impl Into<String>) -> Self {
        Self::Usage(message.into())
    }

    /// 稳定的机器可读错误类别。
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Api(_) => "api",
            Self::Client(_) => "client",
            Self::Usage(_) => "usage",
        }
    }
}

pub type Result<T> = std::result::Result<T, CliError>;
