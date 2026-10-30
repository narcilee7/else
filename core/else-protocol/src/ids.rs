//! 强类型 ID。不同实体的 ID 不可混用，这是编译期保证。

use serde::{Deserialize, Serialize};

macro_rules! define_id {
    ($name:ident, $doc:literal) => {
        #[doc = $doc]
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(pub String);

        impl $name {
            pub fn new() -> Self {
                Self(uuid::Uuid::new_v4().to_string())
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl From<String> for $name {
            fn from(s: String) -> Self {
                Self(s)
            }
        }

        impl From<&str> for $name {
            fn from(s: &str) -> Self {
                Self(s.to_string())
            }
        }
    };
}

define_id!(IntentId, "用户目标。跨执行、跨重启持久。");
define_id!(ScopeId, "执行 Scope：取消与权限传播的单位，树形组织。");
define_id!(RunId, "一次 Agent Attempt。同一 Scope 可重试出多个 Run。");
define_id!(ToolCallId, "一次工具调用，携带幂等键。");
define_id!(AttentionId, "一次注意力请求 / 待本人决策。");
define_id!(
    AttentionWindowId,
    "一次注意力聚合或延迟窗口，窗口截止前不进入用户表面。"
);
define_id!(TimerId, "一次持久化唤醒定时器。");
define_id!(
    CheckpointId,
    "检查点：事件水位 + 工件快照 + 在途副作用结论。"
);
define_id!(ArtifactId, "工件：代码、文档、结构化记录或外部实体的句柄。");
define_id!(LeaseId, "资源租约。");
define_id!(
    SnapshotId,
    "Context Snapshot：一次执行使用的一致上下文版本。"
);
define_id!(MemoryId, "一条记忆记录（候选或已晋升）。");
