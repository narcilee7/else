//! Runtime 级持久策略设置。
//!
//! 这些设置属于 Runtime，而不是某一个 Intent：它们在所有 surface 上
//! 生效，因此定义在协议层，客户端与 Runtime 共用同一形状。

use serde::{Deserialize, Serialize};

/// Runtime 策略设置。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeSettings {
    /// `None` 关闭自动清理。启用后按天清理终态 Intent。
    #[serde(default)]
    pub retention_days: Option<u64>,
}
