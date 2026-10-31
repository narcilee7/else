//! 统一 Capability 与双向 Agent Binding 协议。

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::effect::SideEffectKind;
use crate::ids::{AgentBindingId, AgentId, CapabilityId, RunId, ScopeId};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityKind {
    BuiltinTool,
    McpTool,
    Skill,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CapabilityManifest {
    pub id: CapabilityId,
    pub name: String,
    pub description: String,
    pub kind: CapabilityKind,
    pub effect: SideEffectKind,
    pub idempotency_required: bool,
    #[serde(default)]
    pub input_schema: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SkillManifest {
    pub instructions: String,
    #[serde(default)]
    pub resources: Vec<String>,
    /// Skill 只能引用 Capability，不继承或授予其中权限。
    #[serde(default)]
    pub capability_ids: Vec<CapabilityId>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct McpToolBinding {
    /// MCP Server 启动命令；调用时在 Sandbox 内执行。
    pub command: String,
    pub tool_name: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum CapabilityAdapter {
    Builtin,
    Mcp(McpToolBinding),
    Skill(SkillManifest),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CapabilityRegistration {
    pub manifest: CapabilityManifest,
    pub adapter: CapabilityAdapter,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum AgentTransport {
    LocalProcess { command: String },
    McpStdio { command: String },
    RemoteHttp { endpoint: String },
}

/// Runtime→Agent 的出站 transport 与 Agent→Runtime 的入站 session 同时存在，
/// 共享 Scope/Run/Generation 和 Capability 子集；撤销任一方即整体失效。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentBinding {
    pub id: AgentBindingId,
    pub agent_id: AgentId,
    pub name: String,
    pub scope_id: ScopeId,
    pub run_id: Option<RunId>,
    pub generation: u64,
    pub outbound: AgentTransport,
    pub inbound_session: String,
    pub capability_ids: Vec<CapabilityId>,
    pub expires_at: DateTime<Utc>,
    pub active: bool,
}
