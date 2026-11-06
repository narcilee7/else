//! Capability Registry 与双向 Agent Binding 管理。

use std::collections::BTreeSet;

use chrono::{Duration, Utc};
use else_protocol::capability::{
    AgentBinding, AgentBindingState, AgentTransport, CapabilityAdapter, CapabilityKind,
    CapabilityManifest, CapabilityRegistration, McpServerRegistration, McpToolBinding,
    SkillManifest,
};
use else_protocol::effect::SideEffectKind;
use else_protocol::event::{Actor, EventEnvelope, EventPayload};
use else_protocol::ids::{
    AgentBindingId, AgentId, CapabilityId, McpServerId, RunId, ScopeId, ThreadId,
};

use crate::{ApiError, Result, Runtime};

impl Runtime {
    pub fn discover_mcp_tools(
        &self,
        thread_id: &ThreadId,
        server_name: &str,
        command: &str,
    ) -> Result<Vec<CapabilityRegistration>> {
        let server_id = McpServerId::from(format!(
            "mcp/{server_name}/{:016x}",
            stable_hash(server_name, command)
        ));
        let projection = self.projection(thread_id)?;
        let server = McpServerRegistration {
            id: server_id.clone(),
            name: server_name.into(),
            command: command.into(),
            refresh_interval_seconds: 60,
            enabled: true,
        };
        if projection.mcp_servers.get(&server_id) != Some(&server) {
            let (root, generation, revision) = self.kernel_context(thread_id)?;
            self.ledger.append(&EventEnvelope::new(
                thread_id.clone(),
                root,
                generation,
                revision,
                Actor::User,
                EventPayload::McpServerRegistered {
                    server: server.clone(),
                },
            ))?;
        }
        let mcp_workspace = self.workspace_of(&projection)?;
        let result = else_gateway::mcp_session::list_tools(&mcp_workspace, command)
            .map_err(ApiError::NotFound)?;
        let tools = result["tools"].as_array().cloned().unwrap_or_default();
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
                    server_id: Some(server_id.clone()),
                    command: command.into(),
                    tool_name: tool_name.into(),
                }),
            };
            self.register_capability(thread_id, registration.clone())?;
            registrations.push(registration);
        }
        let capability_ids = registrations
            .iter()
            .map(|registration| registration.manifest.id.clone())
            .collect();
        let (root, generation, revision) = self.kernel_context(thread_id)?;
        self.ledger.append(&EventEnvelope::new(
            thread_id.clone(),
            root,
            generation,
            revision,
            Actor::System,
            EventPayload::McpToolsRefreshed {
                server_id,
                capability_ids,
            },
        ))?;
        Ok(registrations)
    }

    pub fn capabilities(&self, thread_id: &ThreadId) -> Result<Vec<CapabilityRegistration>> {
        let projection = self.projection(thread_id)?;
        let mut capabilities = else_gateway::tool::builtin_capabilities();
        capabilities.extend(projection.capabilities.into_values());
        Ok(capabilities)
    }

    pub fn agent_bindings(&self, thread_id: &ThreadId) -> Result<Vec<AgentBinding>> {
        Ok(self
            .projection(thread_id)?
            .agent_bindings
            .into_values()
            .filter(|binding| binding.active && binding.expires_at > Utc::now())
            .collect())
    }

    pub fn register_skill(
        &self,
        thread_id: &ThreadId,
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
        self.register_capability(thread_id, registration.clone())?;
        Ok(registration)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn register_mcp_tool(
        &self,
        thread_id: &ThreadId,
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
                server_id: None,
                command: command.into(),
                tool_name: tool_name.into(),
            }),
        };
        self.register_capability(thread_id, registration.clone())?;
        Ok(registration)
    }

    pub fn register_capability(
        &self,
        thread_id: &ThreadId,
        capability: CapabilityRegistration,
    ) -> Result<()> {
        let (root, generation, revision) = self.kernel_context(thread_id)?;
        self.ledger.append(&EventEnvelope::new(
            thread_id.clone(),
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
        thread_id: &ThreadId,
        agent_id: AgentId,
        name: &str,
        scope_id: ScopeId,
        run_id: Option<RunId>,
        outbound: AgentTransport,
        capability_ids: Vec<CapabilityId>,
        ttl_seconds: i64,
    ) -> Result<AgentBinding> {
        let projection = self.projection(thread_id)?;
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
            state: AgentBindingState::Active,
        };
        let revision = projection.spec.as_ref().map_or(1, |spec| spec.revision);
        self.ledger.append(&EventEnvelope::new(
            thread_id.clone(),
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
        thread_id: &ThreadId,
        binding_id: &AgentBindingId,
        reason: &str,
    ) -> Result<()> {
        let projection = self.projection(thread_id)?;
        let binding = projection
            .agent_bindings
            .get(binding_id)
            .ok_or_else(|| ApiError::NotFound(format!("binding {binding_id}")))?;
        let revision = projection.spec.as_ref().map_or(1, |spec| spec.revision);
        self.ledger.append(&EventEnvelope::new(
            thread_id.clone(),
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

    /// Runtime 启动时重建所有持久 MCP Server 的 stdio session。
    pub fn restore_mcp_sessions(&self) -> Result<()> {
        for spec in self.list_threads()? {
            let projection = self.projection(&spec.id)?;
            let workspace = self.workspace_of(&projection)?;
            for server in projection
                .mcp_servers
                .values()
                .filter(|server| server.enabled)
            {
                else_gateway::mcp_session::list_tools(&workspace, &server.command)
                    .map_err(ApiError::NotFound)?;
            }
        }
        Ok(())
    }

    /// 处理 MCP list_changed：刷新 dirty server，新增/移除 Capability。
    pub fn refresh_dirty_mcp_servers(&self) -> Result<()> {
        for spec in self.list_threads()? {
            let projection = self.projection(&spec.id)?;
            for server_id in projection.dirty_mcp_servers {
                let Some(server) = projection.mcp_servers.get(&server_id) else {
                    continue;
                };
                self.discover_mcp_tools(&spec.id, &server.name, &server.command)?;
            }
        }
        Ok(())
    }
}

fn stable_hash(left: &str, right: &str) -> u64 {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in left.bytes().chain([0]).chain(right.bytes()) {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}
