//! Capability Registry 与双向 Agent Binding 管理。

use std::collections::BTreeSet;

use chrono::{Duration, Utc};
use else_protocol::capability::{
    AgentBinding, AgentTransport, CapabilityAdapter, CapabilityKind, CapabilityManifest,
    CapabilityRegistration, McpToolBinding, SkillManifest,
};
use else_protocol::effect::SideEffectKind;
use else_protocol::event::{Actor, EventEnvelope, EventPayload};
use else_protocol::ids::{AgentBindingId, AgentId, CapabilityId, IntentId, RunId, ScopeId};

use crate::{ApiError, Result, Runtime};

impl Runtime {
    pub fn discover_mcp_tools(
        &self,
        intent_id: &IntentId,
        server_name: &str,
        command: &str,
    ) -> Result<Vec<CapabilityRegistration>> {
        let requests = format!(
            "{}\n{}\n{}\n",
            serde_json::json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "initialize",
                "params": {
                    "protocolVersion": "2025-03-26",
                    "capabilities": {},
                    "clientInfo": {"name": "else", "version": "0.1.0"}
                }
            }),
            serde_json::json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
            serde_json::json!({"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}),
        );
        let sandbox =
            else_environment::Sandbox::new(else_environment::SandboxProfile::new(&self.workspace))
                .map_err(|error| ApiError::NotFound(error.to_string()))?;
        let output = sandbox
            .run_shell_with_input(command, &requests)
            .map_err(|error| ApiError::NotFound(error.to_string()))?;
        let result = String::from_utf8_lossy(&output.stdout)
            .lines()
            .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
            .find(|value| value["id"] == 2)
            .ok_or_else(|| ApiError::NotFound("MCP tools/list response".into()))?;
        let tools = result["result"]["tools"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        let mut registrations = Vec::new();
        for tool in tools {
            let Some(tool_name) = tool["name"].as_str() else {
                continue;
            };
            let effect = if tool["annotations"]["readOnlyHint"].as_bool() == Some(true) {
                SideEffectKind::ReadOnly
            } else if tool["annotations"]["destructiveHint"].as_bool() == Some(true) {
                SideEffectKind::Irreversible
            } else {
                SideEffectKind::Unknown
            };
            let registration = CapabilityRegistration {
                manifest: CapabilityManifest {
                    id: CapabilityId::from(format!("mcp/{server_name}/{tool_name}")),
                    name: format!("{server_name}:{tool_name}"),
                    description: tool["description"].as_str().unwrap_or("MCP tool").into(),
                    kind: CapabilityKind::McpTool,
                    effect,
                    idempotency_required: effect != SideEffectKind::ReadOnly,
                    input_schema: tool["inputSchema"].clone(),
                },
                adapter: CapabilityAdapter::Mcp(McpToolBinding {
                    command: command.into(),
                    tool_name: tool_name.into(),
                }),
            };
            self.register_capability(intent_id, registration.clone())?;
            registrations.push(registration);
        }
        Ok(registrations)
    }

    pub fn capabilities(&self, intent_id: &IntentId) -> Result<Vec<CapabilityRegistration>> {
        let projection = self.projection(intent_id)?;
        let mut capabilities = else_gateway::tool::builtin_capabilities();
        capabilities.extend(projection.capabilities.into_values());
        Ok(capabilities)
    }

    pub fn agent_bindings(&self, intent_id: &IntentId) -> Result<Vec<AgentBinding>> {
        Ok(self
            .projection(intent_id)?
            .agent_bindings
            .into_values()
            .filter(|binding| binding.active && binding.expires_at > Utc::now())
            .collect())
    }

    pub fn register_skill(
        &self,
        intent_id: &IntentId,
        name: &str,
        description: &str,
        instructions: &str,
        resources: Vec<String>,
        capability_ids: Vec<CapabilityId>,
    ) -> Result<CapabilityRegistration> {
        let registration = CapabilityRegistration {
            manifest: CapabilityManifest {
                id: CapabilityId::new(),
                name: name.into(),
                description: description.into(),
                kind: CapabilityKind::Skill,
                effect: SideEffectKind::ReadOnly,
                idempotency_required: false,
                input_schema: serde_json::json!({"type":"object"}),
            },
            adapter: CapabilityAdapter::Skill(SkillManifest {
                instructions: instructions.into(),
                resources,
                capability_ids,
            }),
        };
        self.register_capability(intent_id, registration.clone())?;
        Ok(registration)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn register_mcp_tool(
        &self,
        intent_id: &IntentId,
        name: &str,
        description: &str,
        command: &str,
        tool_name: &str,
        effect: SideEffectKind,
        input_schema: serde_json::Value,
    ) -> Result<CapabilityRegistration> {
        let registration = CapabilityRegistration {
            manifest: CapabilityManifest {
                id: CapabilityId::new(),
                name: name.into(),
                description: description.into(),
                kind: CapabilityKind::McpTool,
                effect,
                idempotency_required: true,
                input_schema,
            },
            adapter: CapabilityAdapter::Mcp(McpToolBinding {
                command: command.into(),
                tool_name: tool_name.into(),
            }),
        };
        self.register_capability(intent_id, registration.clone())?;
        Ok(registration)
    }

    pub fn register_capability(
        &self,
        intent_id: &IntentId,
        capability: CapabilityRegistration,
    ) -> Result<()> {
        let (root, generation, revision) = self.kernel_context(intent_id)?;
        self.ledger.append(&EventEnvelope::new(
            intent_id.clone(),
            root,
            generation,
            revision,
            Actor::User,
            EventPayload::CapabilityRegistered { capability },
        ))?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub fn bind_agent(
        &self,
        intent_id: &IntentId,
        agent_id: AgentId,
        name: &str,
        scope_id: ScopeId,
        run_id: Option<RunId>,
        outbound: AgentTransport,
        capability_ids: Vec<CapabilityId>,
        ttl_seconds: i64,
    ) -> Result<AgentBinding> {
        let projection = self.projection(intent_id)?;
        let scope = projection
            .scopes
            .get(&scope_id)
            .ok_or_else(|| ApiError::NotFound(format!("scope {scope_id}")))?;
        let known: BTreeSet<_> = else_gateway::tool::builtin_capabilities()
            .into_iter()
            .map(|capability| capability.manifest.id)
            .chain(projection.capabilities.keys().cloned())
            .collect();
        if let Some(capability) = capability_ids.iter().find(|id| !known.contains(*id)) {
            return Err(ApiError::NotFound(format!("capability {capability}")));
        }
        let binding = AgentBinding {
            id: AgentBindingId::new(),
            agent_id,
            name: name.into(),
            scope_id: scope_id.clone(),
            run_id,
            generation: scope.generation,
            outbound,
            inbound_session: format!("session-{}", uuid::Uuid::new_v4()),
            capability_ids,
            expires_at: Utc::now() + Duration::seconds(ttl_seconds.max(1)),
            active: true,
        };
        let revision = projection.spec.as_ref().map_or(1, |spec| spec.revision);
        self.ledger.append(&EventEnvelope::new(
            intent_id.clone(),
            scope_id,
            scope.generation,
            revision,
            Actor::User,
            EventPayload::AgentBindingCreated {
                binding: binding.clone(),
            },
        ))?;
        Ok(binding)
    }

    pub fn revoke_agent_binding(
        &self,
        intent_id: &IntentId,
        binding_id: &AgentBindingId,
        reason: &str,
    ) -> Result<()> {
        let projection = self.projection(intent_id)?;
        let binding = projection
            .agent_bindings
            .get(binding_id)
            .ok_or_else(|| ApiError::NotFound(format!("binding {binding_id}")))?;
        let revision = projection.spec.as_ref().map_or(1, |spec| spec.revision);
        self.ledger.append(&EventEnvelope::new(
            intent_id.clone(),
            binding.scope_id.clone(),
            binding.generation,
            revision,
            Actor::User,
            EventPayload::AgentBindingRevoked {
                binding_id: binding_id.clone(),
                reason: reason.into(),
            },
        ))?;
        Ok(())
    }
}
