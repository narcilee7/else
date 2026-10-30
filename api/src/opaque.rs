//! Opaque Harness Profile：显式用户风险授权与可接管能力检查。

use else_gateway::GatewayError;
use else_harness::adapter::HarnessAdapter;
use else_harness::egress::EgressProxy;
use else_protocol::event::{Actor, EventEnvelope, EventPayload};
use else_protocol::harness::{HarnessNetworkAccess, ToolEffectVisibility};
use else_protocol::ids::IntentId;
use else_protocol::intent::{IntentSpec, OpaqueHarnessPolicy};

use crate::{ApiError, Result, Runtime};

impl Runtime {
    /// 用户显式启用 Opaque Profile。该操作提高 Intent Revision，但不扩大
    /// `Authority`；只允许观察上限，不等于给 Harness 签发工具权限。
    pub fn configure_opaque_harness(
        &self,
        intent_id: &IntentId,
        policy: OpaqueHarnessPolicy,
    ) -> Result<IntentSpec> {
        let projection = self.projection(intent_id)?;
        for (scope_id, scope) in &projection.scopes {
            if matches!(scope.state, else_protocol::event::ScopeState::Running)
                && self.active_run_id(intent_id, scope_id)?.is_some()
            {
                return Err(ApiError::Conflict(
                    "cannot change Opaque Profile while a Run is active".into(),
                ));
            }
        }
        let mut spec = projection
            .spec
            .clone()
            .ok_or_else(|| ApiError::IntentNotFound(intent_id.clone()))?;
        spec.revision = spec.revision.saturating_add(1);
        spec.opaque_harness_policy = policy;
        let root = projection
            .root_scope_id
            .ok_or_else(|| ApiError::IntentNotFound(intent_id.clone()))?;
        let generation = projection
            .scopes
            .get(&root)
            .map(|scope| scope.generation)
            .unwrap_or(1);
        self.ledger.append(&EventEnvelope::new(
            intent_id.clone(),
            root,
            generation,
            spec.revision,
            Actor::User,
            EventPayload::IntentRevised {
                spec: spec.clone(),
                reason: "用户配置 Opaque Harness Profile".into(),
            },
        ))?;
        Ok(spec)
    }

    pub(crate) fn ensure_harness_allowed(
        &self,
        spec: &IntentSpec,
        adapter: &dyn HarnessAdapter,
    ) -> Result<()> {
        let capabilities = adapter.capabilities();
        let network = adapter.network_access();
        let opaque_profile = capabilities.tool_effects == ToolEffectVisibility::Opaque
            || network != HarnessNetworkAccess::Deny;
        if !opaque_profile {
            return Ok(());
        }
        let policy = spec.opaque_harness_policy;
        if !policy.enabled {
            return Err(GatewayError::OpaqueHarnessDenied(format!(
                "{} requires an explicit Opaque Profile",
                adapter.name()
            ))
            .into());
        }
        if network != HarnessNetworkAccess::Deny && !policy.allow_native_provider_network {
            return Err(GatewayError::OpaqueHarnessDenied(format!(
                "{} requests native provider network but the Intent did not approve it",
                adapter.name()
            ))
            .into());
        }
        Ok(())
    }

    pub(crate) fn harness_allowed(&self, spec: &IntentSpec, adapter: &dyn HarnessAdapter) -> bool {
        self.ensure_harness_allowed(spec, adapter).is_ok()
    }

    pub(crate) fn start_egress_proxy(
        &self,
        adapter: &dyn HarnessAdapter,
    ) -> Result<Option<EgressProxy>> {
        if adapter.network_access() == HarnessNetworkAccess::Deny {
            return Ok(None);
        }
        let allowlist = adapter
            .egress_allowlist()
            .into_iter()
            .map(str::to_string)
            .collect();
        EgressProxy::start(allowlist).map(Some).map_err(|error| {
            GatewayError::OpaqueHarnessDenied(format!(
                "failed to start egress proxy for {}: {error}",
                adapter.name()
            ))
            .into()
        })
    }

    pub(crate) fn record_egress_decisions(
        &self,
        ctx: &else_gateway::ExecContext,
        proxy: Option<&EgressProxy>,
    ) -> Result<()> {
        let Some(proxy) = proxy else {
            return Ok(());
        };
        for decision in proxy.decisions() {
            ctx.record(
                &self.ledger,
                Actor::System,
                EventPayload::EgressIntercepted { decision },
            )?;
        }
        Ok(())
    }
}
