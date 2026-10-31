//! Outbound Agent dispatch over a persisted AgentBinding.

use else_harness::adapter::{
    shell_quote, BridgeEvent, HarnessPrep, LineParser, PrepareFn, ProcessHarness,
};
use else_ledger::IntentProjection;
use else_protocol::capability::{AgentBindingEvent, AgentBindingState, AgentTransport};
use else_protocol::effect::SideEffectKind;
use else_protocol::event::{Actor, EventEnvelope, EventPayload};
use else_protocol::harness::{
    CancelSupport, HarnessCapabilities, ToolEffectVisibility, UsageSupport,
};
use else_protocol::ids::{AgentBindingId, IntentId};
use else_protocol::resource::{ModelUsage, UsageBasis};

use crate::{ApiError, Result, Runtime};

struct BindingParser;

impl LineParser for BindingParser {
    fn parse(&self, line: &str) -> Vec<BridgeEvent> {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
            return vec![];
        };
        match value["type"].as_str() {
            Some("output") => vec![BridgeEvent::Output {
                text: value["text"].as_str().unwrap_or_default().into(),
            }],
            Some("tool") => vec![BridgeEvent::ToolObserved {
                tool: value["tool"].as_str().unwrap_or("unknown").into(),
                effect: serde_json::from_value(value["effect"].clone())
                    .unwrap_or(SideEffectKind::Unknown),
                summary: value["summary"].as_str().unwrap_or_default().into(),
                success: value["success"].as_bool().unwrap_or(true),
            }],
            Some("artifact") => vec![BridgeEvent::Artifact {
                locator: value["locator"].as_str().unwrap_or_default().into(),
                note: value["note"]
                    .as_str()
                    .unwrap_or("bound agent artifact")
                    .into(),
            }],
            Some("usage") => vec![BridgeEvent::Usage {
                model: value["model"].as_str().unwrap_or("bound-agent").into(),
                usage: ModelUsage {
                    input_tokens: value["input_tokens"].as_u64().unwrap_or(0),
                    output_tokens: value["output_tokens"].as_u64().unwrap_or(0),
                    cost_usd_micros: value["cost_usd_micros"].as_u64().unwrap_or(0),
                    basis: UsageBasis::Reported,
                },
            }],
            Some("completed") => vec![BridgeEvent::Completed {
                summary: value["summary"].as_str().unwrap_or_default().into(),
            }],
            Some("failed") => vec![BridgeEvent::Failed {
                reason: value["reason"]
                    .as_str()
                    .unwrap_or("bound agent failed")
                    .into(),
            }],
            _ => vec![],
        }
    }
}

impl Runtime {
    pub fn dispatch_agent_binding(
        &self,
        intent_id: &IntentId,
        binding_id: &AgentBindingId,
    ) -> Result<IntentProjection> {
        let projection = self.projection(intent_id)?;
        let binding = projection
            .agent_bindings
            .get(binding_id)
            .ok_or_else(|| ApiError::NotFound(format!("binding {binding_id}")))?;
        if !binding.active || binding.expires_at <= chrono::Utc::now() {
            return Err(ApiError::Conflict(format!(
                "binding {binding_id} is inactive"
            )));
        }
        let capabilities = binding.capability_ids.clone();
        let prompt_capabilities = capabilities.clone();
        let binding_session = binding.inbound_session.clone();
        let (prepare, network, egress_allowlist): (
            PrepareFn,
            else_environment::NetworkPolicy,
            &'static [&'static str],
        ) = match binding.outbound.clone() {
            AgentTransport::LocalProcess { command } => {
                let session = binding_session.clone();
                let allowed = prompt_capabilities.clone();
                (
                    Box::new(move |_workspace, _private, prompt| {
                        let dispatch = format!(
                            "{prompt}\n\n# Agent Binding\nsession={session}\ncapabilities={allowed:?}"
                        );
                        Ok(HarnessPrep {
                            command: format!("{} {}", command, shell_quote(&dispatch)),
                            env: vec![],
                            read_paths: vec![],
                            write_paths: vec![],
                            cleanup_paths: vec![],
                        })
                    }),
                    else_environment::NetworkPolicy::Deny,
                    &[],
                )
            }
            AgentTransport::McpStdio { command, tool_name } => {
                let tool_name = tool_name.ok_or_else(|| {
                    ApiError::NotFound("McpStdio binding requires tool_name".into())
                })?;
                let session = binding_session.clone();
                (
                    Box::new(move |workspace, _private, prompt| {
                        else_gateway::mcp_session::call_tool(
                            workspace,
                            &command,
                            &tool_name,
                            serde_json::json!({"task":prompt,"session":session}),
                        )
                        .map_err(std::io::Error::other)?;
                        Ok(HarnessPrep {
                            command: "printf '%s\\n' '{\"type\":\"completed\",\"summary\":\"mcp binding done\"}'".into(),
                            env: vec![],
                            read_paths: vec![],
                            write_paths: vec![],
                            cleanup_paths: vec![],
                        })
                    }),
                    else_environment::NetworkPolicy::Deny,
                    &[],
                )
            }
            AgentTransport::RemoteHttp {
                endpoint,
                callback_url,
            } => {
                let host = endpoint
                    .strip_prefix("https://")
                    .and_then(|rest| rest.split('/').next())
                    .and_then(|authority| authority.split(':').next())
                    .ok_or_else(|| {
                        ApiError::NotFound("RemoteHttp binding requires an https endpoint".into())
                    })?;
                let static_host: &'static str = Box::leak(host.to_string().into_boxed_str());
                let allowlist: &'static [&'static str] =
                    Box::leak(vec![static_host].into_boxed_slice());
                let session = binding_session.clone();
                let allowed = prompt_capabilities.clone();
                (
                    Box::new(move |_workspace, _private, prompt| {
                        let body = serde_json::json!({
                            "task": prompt,
                            "inbound_session": session,
                            "capability_ids": allowed,
                            "callback_url": callback_url,
                        })
                        .to_string();
                        Ok(HarnessPrep {
                            command: format!(
                                "curl -sS --fail --max-time 900 -X POST \
                                 -H 'Content-Type: application/json' \
                                 -H {} --data {} {}",
                                shell_quote(&format!("Authorization: Bearer {session}")),
                                shell_quote(&body),
                                shell_quote(&endpoint)
                            ),
                            env: vec![],
                            read_paths: vec![],
                            write_paths: vec![],
                            cleanup_paths: vec![],
                        })
                    }),
                    else_environment::NetworkPolicy::AllowOutbound,
                    allowlist,
                )
            }
        };
        let harness = ProcessHarness {
            name: "bound-agent",
            capabilities: HarnessCapabilities {
                cancel: CancelSupport::KillOnly,
                checkpoint_resume: false,
                usage: UsageSupport::Reported,
                tool_effects: ToolEffectVisibility::Opaque,
                interactive_input: false,
            },
            network,
            egress_allowlist,
            timeout: Some(std::time::Duration::from_secs(15 * 60)),
            prepare,
            parser: BindingParser,
        };
        self.dispatch_scope_adapter_with_capabilities(
            intent_id,
            &binding.scope_id,
            &harness,
            Some(capabilities),
        )
    }

    pub fn record_agent_binding_event(
        &self,
        intent_id: &IntentId,
        binding_id: &AgentBindingId,
        session: &str,
        event: AgentBindingEvent,
    ) -> Result<IntentProjection> {
        let projection = self.projection(intent_id)?;
        let binding = projection
            .agent_bindings
            .get(binding_id)
            .ok_or_else(|| ApiError::NotFound(format!("binding {binding_id}")))?;
        if !binding.active
            || binding.state == AgentBindingState::Closed
            || binding.inbound_session != session
        {
            return Err(ApiError::Conflict("binding session is not active".into()));
        }
        let revision = projection.spec.as_ref().map_or(1, |spec| spec.revision);
        self.ledger.append(&EventEnvelope::new(
            intent_id.clone(),
            binding.scope_id.clone(),
            binding.generation,
            revision,
            Actor::External,
            EventPayload::AgentBindingEventReceived {
                binding_id: binding_id.clone(),
                event,
            },
        ))?;
        self.projection(intent_id)
    }

    pub fn transition_agent_binding(
        &self,
        intent_id: &IntentId,
        binding_id: &AgentBindingId,
        state: AgentBindingState,
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
            EventPayload::AgentBindingStateChanged {
                binding_id: binding_id.clone(),
                state,
            },
        ))?;
        Ok(())
    }
}
