//! Scope 失败分类：区分可重试故障与不应重复执行的终态故障。

use serde::{Deserialize, Serialize};

use crate::ids::RunId;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureClass {
    Retryable,
    Terminal,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScopeFailure {
    pub run_id: RunId,
    pub generation: u64,
    pub class: FailureClass,
    pub code: String,
    pub message: String,
    pub attempt: u32,
}
