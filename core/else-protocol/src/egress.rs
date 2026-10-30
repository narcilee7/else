//! Opaque Harness 出口决策。代理记录域名判断，不保存 TLS 内容。

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EgressDecision {
    pub host: String,
    pub allowed: bool,
    pub reason: String,
}
