//! Provider 外部写动作：提案、一次性授权、Tool Gateway 执行与 Unknown 收敛。

use else_gateway::{ExecContext, ExternalActionExecutor, GatewayError, ToolGateway};
use else_protocol::capability::{CapabilityAdapter, ExternalActionBinding};
use else_protocol::connector::{
    ConnectorAction, ConnectorActionPreview, ConnectorActionStatus, ConnectorCredential,
    ConnectorRegistration,
};
use else_protocol::effect::{EffectConclusion, ToolCall, ToolOutcome, ToolStatus};
use else_protocol::event::{Actor, EventEnvelope, EventPayload};
use else_protocol::ids::{ConnectorActionId, ConnectorId, IntentId, RunId, ToolCallId};
use serde_json::{json, Value};

use crate::{ApiError, Result, Runtime};

impl Runtime {
    pub fn preview_connector_action(
        &self,
        intent_id: &IntentId,
        connector_id: &ConnectorId,
        capability: &str,
        target: &str,
        input: &Value,
    ) -> Result<ConnectorActionPreview> {
        if capability.trim().is_empty() || target.trim().is_empty() {
            return Err(ApiError::Conflict(
                "connector action capability and target cannot be empty".into(),
            ));
        }
        let projection = self.projection(intent_id)?;
        let connector = projection
            .connectors
            .get(connector_id)
            .cloned()
            .ok_or_else(|| ApiError::NotFound(format!("connector {connector_id}")))?;
        if !connector
            .manifest
            .write_capabilities
            .iter()
            .any(|declared| declared == capability)
        {
            return Err(ApiError::Conflict(format!(
                "connector {connector_id} does not declare write capability {capability}"
            )));
        }
        let adapter = self
            .connector_adapters
            .get(&connector.adapter)
            .ok_or_else(|| {
                ApiError::NotFound(format!("connector adapter {}", connector.adapter))
            })?;
        let credential = self.connector_credential(&connector)?;
        let current = adapter
            .preview_action(
                &connector.source,
                capability,
                target,
                credential.as_ref(),
                input,
            )
            .map_err(|error| ApiError::Conflict(error.to_string()))?;
        let proposed = input
            .get("changes")
            .cloned()
            .unwrap_or_else(|| input.clone());
        Ok(ConnectorActionPreview {
            connector_id: connector_id.clone(),
            capability: capability.into(),
            target: target.into(),
            current,
            proposed,
            checked_at: chrono::Utc::now(),
        })
    }

    pub fn propose_connector_action(
        &self,
        intent_id: &IntentId,
        connector_id: &ConnectorId,
        capability: &str,
        target: &str,
        input: Value,
        idempotency_key: Option<String>,
    ) -> Result<ConnectorAction> {
        if capability.trim().is_empty() || target.trim().is_empty() {
            return Err(ApiError::Conflict(
                "connector action capability and target cannot be empty".into(),
            ));
        }
        let projection = self.projection(intent_id)?;
        let connector = projection
            .connectors
            .get(connector_id)
            .cloned()
            .ok_or_else(|| ApiError::NotFound(format!("connector {connector_id}")))?;
        if !connector
            .manifest
            .write_capabilities
            .iter()
            .any(|declared| declared == capability)
        {
            return Err(ApiError::Conflict(format!(
                "connector {connector_id} does not declare write capability {capability}"
            )));
        }
        self.register_connector_write_capabilities(intent_id, &connector)?;
        let projection = self.projection(intent_id)?;
        let action_capability = projection
            .capabilities
            .values()
            .find(|registration| {
                matches!(
                    &registration.adapter,
                    CapabilityAdapter::ExternalAction(binding)
                        if binding.connector_id == *connector_id
                            && binding.operation == capability
                )
            })
            .ok_or_else(|| {
                ApiError::Conflict(format!(
                    "connector {connector_id} capability {capability} was not registered"
                ))
            })?;
        let id = ConnectorActionId::new();
        let action = ConnectorAction {
            id: id.clone(),
            connector_id: connector_id.clone(),
            capability: capability.into(),
            target: target.into(),
            input,
            effect: action_capability.manifest.effect,
            idempotency_key: idempotency_key
                .filter(|key| !key.trim().is_empty())
                .unwrap_or_else(|| format!("connector-action:{id}")),
            status: ConnectorActionStatus::Proposed,
            proposed_at: chrono::Utc::now(),
            attention_id: None,
            tool_call_id: None,
            outcome: None,
            last_error: None,
        };
        let (root, generation, revision) = self.kernel_context(intent_id)?;
        self.ledger.append(&EventEnvelope::new(
            intent_id.clone(),
            root,
            generation,
            revision,
            Actor::User,
            EventPayload::ConnectorActionProposed {
                action: action.clone(),
            },
        ))?;
        Ok(action)
    }

    pub fn execute_connector_action(
        &self,
        intent_id: &IntentId,
        action_id: &ConnectorActionId,
    ) -> Result<ConnectorAction> {
        let projection = self.projection(intent_id)?;
        let action = projection
            .connector_actions
            .get(action_id)
            .cloned()
            .ok_or_else(|| ApiError::NotFound(format!("connector action {action_id}")))?;
        match action.status {
            ConnectorActionStatus::Applied => return Ok(action),
            ConnectorActionStatus::Executing => {
                // 进程可能死在 Provider 已成功、ToolCompleted 尚未落账之间。
                // 没有 Provider 查询结果前不能再次执行。
                self.record_action_state(
                    intent_id,
                    &action,
                    ConnectorActionStatus::Unknown,
                    None,
                    action.tool_call_id.clone(),
                    None,
                    Some("execution was interrupted; reconcile with provider first".into()),
                )?;
                return self.projected_action(intent_id, action_id);
            }
            ConnectorActionStatus::Unknown
            | ConnectorActionStatus::Failed
            | ConnectorActionStatus::Compensating
            | ConnectorActionStatus::CompensationExecuting
            | ConnectorActionStatus::Compensated
            | ConnectorActionStatus::CompensationFailed
            | ConnectorActionStatus::Cancelled => {
                return Err(ApiError::Conflict(format!(
                    "connector action {action_id} is {:?}; create a new action instead of retrying",
                    action.status
                )))
            }
            ConnectorActionStatus::Proposed | ConnectorActionStatus::NeedsApproval => {}
        }

        let connector = projection
            .connectors
            .get(&action.connector_id)
            .cloned()
            .ok_or_else(|| ApiError::NotFound(format!("connector {}", action.connector_id)))?;
        let adapter = self
            .connector_adapters
            .get(&connector.adapter)
            .ok_or_else(|| {
                ApiError::NotFound(format!("connector adapter {}", connector.adapter))
            })?;
        let credential = self.connector_credential(&connector)?;
        let root_scope = projection
            .root_scope_id
            .ok_or_else(|| ApiError::IntentNotFound(intent_id.clone()))?;
        let generation = projection
            .scopes
            .get(&root_scope)
            .map(|scope| scope.generation)
            .unwrap_or(1);
        let spec = projection
            .spec
            .clone()
            .ok_or_else(|| ApiError::IntentNotFound(intent_id.clone()))?;
        let ctx = ExecContext {
            spec,
            scope_id: root_scope,
            run_id: RunId::new(),
            generation,
            snapshot_id: None,
        };
        let lease = self.admit_external_action(&ctx)?;
        let tool_call_id = ToolCallId::new();
        self.record_action_state(
            intent_id,
            &action,
            ConnectorActionStatus::Executing,
            action.attention_id.clone(),
            Some(tool_call_id.clone()),
            None,
            None,
        )?;
        let executor = AdapterActionExecutor {
            adapter,
            connector: connector.clone(),
            credential,
            mode: ActionMode::Execute,
        };
        let tools = ToolGateway::new(&self.ledger, self.workspace.clone())
            .with_external_executor(&executor);
        let mut input = action.input.clone();
        if let Value::Object(object) = &mut input {
            object
                .entry("target")
                .or_insert_with(|| json!(action.target));
        }
        let call = ToolCall {
            id: tool_call_id.clone(),
            tool: format!("connector:{}:{}", connector.id, action.capability),
            effect: action.effect,
            input,
            idempotency_key: action.idempotency_key.clone(),
        };
        let result = tools.execute(&ctx, call);
        let _ = self.revoke_lease(&ctx, &lease);
        match result {
            Ok(mut outcome) => {
                if let Some(before) = action
                    .outcome
                    .as_ref()
                    .and_then(|previous| previous.output.get("before"))
                {
                    if let Value::Object(output) = &mut outcome.output {
                        output.entry("before").or_insert_with(|| before.clone());
                    }
                }
                let status = match outcome.status {
                    ToolStatus::Succeeded => ConnectorActionStatus::Applied,
                    ToolStatus::Failed => ConnectorActionStatus::Failed,
                    ToolStatus::Unknown => ConnectorActionStatus::Unknown,
                };
                self.record_action_state(
                    intent_id,
                    &action,
                    status,
                    action.attention_id.clone(),
                    Some(tool_call_id),
                    Some(outcome),
                    None,
                )?;
                self.projected_action(intent_id, action_id)
            }
            Err(GatewayError::NeedsApproval(request_id)) => {
                let pending_request = request_id.clone();
                self.record_action_state(
                    intent_id,
                    &action,
                    ConnectorActionStatus::NeedsApproval,
                    Some(request_id),
                    Some(tool_call_id),
                    None,
                    None,
                )?;
                Err(GatewayError::NeedsApproval(pending_request).into())
            }
            Err(error) => {
                self.record_action_state(
                    intent_id,
                    &action,
                    ConnectorActionStatus::Failed,
                    action.attention_id.clone(),
                    Some(tool_call_id),
                    None,
                    Some(error.to_string()),
                )?;
                Err(error.into())
            }
        }
    }

    pub fn connector_actions(&self, intent_id: &IntentId) -> Result<Vec<ConnectorAction>> {
        Ok(self
            .projection(intent_id)?
            .connector_actions
            .into_values()
            .collect())
    }

    /// 对 Executing / Unknown / Applied 动作回源查询 Provider，不产生新的写副作用。
    pub fn verify_connector_action(
        &self,
        intent_id: &IntentId,
        action_id: &ConnectorActionId,
    ) -> Result<ConnectorAction> {
        let projection = self.projection(intent_id)?;
        let action = projection
            .connector_actions
            .get(action_id)
            .cloned()
            .ok_or_else(|| ApiError::NotFound(format!("connector action {action_id}")))?;
        if !matches!(
            action.status,
            ConnectorActionStatus::Executing
                | ConnectorActionStatus::Unknown
                | ConnectorActionStatus::Applied
        ) {
            return Err(ApiError::Conflict(format!(
                "connector action {action_id} cannot be verified from {:?}",
                action.status
            )));
        }
        let connector = projection
            .connectors
            .get(&action.connector_id)
            .cloned()
            .ok_or_else(|| ApiError::NotFound(format!("connector {}", action.connector_id)))?;
        let adapter = self
            .connector_adapters
            .get(&connector.adapter)
            .ok_or_else(|| {
                ApiError::NotFound(format!("connector adapter {}", connector.adapter))
            })?;
        let credential = self.connector_credential(&connector)?;
        match adapter.verify_action(&connector.source, &action, credential.as_ref()) {
            Ok(mut outcome) => {
                if let Some(before) = action
                    .outcome
                    .as_ref()
                    .and_then(|previous| previous.output.get("before"))
                {
                    if let Value::Object(output) = &mut outcome.output {
                        output.entry("before").or_insert_with(|| before.clone());
                    }
                }
                let status = match outcome.status {
                    ToolStatus::Succeeded => ConnectorActionStatus::Applied,
                    ToolStatus::Failed => ConnectorActionStatus::Failed,
                    ToolStatus::Unknown => ConnectorActionStatus::Unknown,
                };
                self.record_action_state(
                    intent_id,
                    &action,
                    status,
                    action.attention_id.clone(),
                    action.tool_call_id.clone(),
                    Some(outcome),
                    None,
                )?;
                self.projected_action(intent_id, action_id)
            }
            Err(error) => {
                self.record_action_state(
                    intent_id,
                    &action,
                    ConnectorActionStatus::Unknown,
                    action.attention_id.clone(),
                    action.tool_call_id.clone(),
                    action.outcome.clone(),
                    Some(error.to_string()),
                )?;
                Err(error.into())
            }
        }
    }

    /// 补偿本身也是外部写，必须重新通过 Tool Gateway 与 Decision。
    pub fn compensate_connector_action(
        &self,
        intent_id: &IntentId,
        action_id: &ConnectorActionId,
    ) -> Result<ConnectorAction> {
        let projection = self.projection(intent_id)?;
        let action = projection
            .connector_actions
            .get(action_id)
            .cloned()
            .ok_or_else(|| ApiError::NotFound(format!("connector action {action_id}")))?;
        match action.status {
            ConnectorActionStatus::Compensated => return Ok(action),
            ConnectorActionStatus::Applied => {}
            ConnectorActionStatus::CompensationExecuting => {
                self.record_action_state(
                    intent_id,
                    &action,
                    ConnectorActionStatus::Unknown,
                    action.attention_id.clone(),
                    action.tool_call_id.clone(),
                    action.outcome.clone(),
                    Some("compensation was interrupted; verify with provider first".into()),
                )?;
                return self.projected_action(intent_id, action_id);
            }
            ConnectorActionStatus::Compensating => {}
            _ => {
                return Err(ApiError::Conflict(format!(
                    "connector action {action_id} cannot be compensated from {:?}",
                    action.status
                )))
            }
        }
        let connector = projection
            .connectors
            .get(&action.connector_id)
            .cloned()
            .ok_or_else(|| ApiError::NotFound(format!("connector {}", action.connector_id)))?;
        let adapter = self
            .connector_adapters
            .get(&connector.adapter)
            .ok_or_else(|| {
                ApiError::NotFound(format!("connector adapter {}", connector.adapter))
            })?;
        let credential = self.connector_credential(&connector)?;
        let root_scope = projection
            .root_scope_id
            .ok_or_else(|| ApiError::IntentNotFound(intent_id.clone()))?;
        let generation = projection
            .scopes
            .get(&root_scope)
            .map(|scope| scope.generation)
            .unwrap_or(1);
        let spec = projection
            .spec
            .clone()
            .ok_or_else(|| ApiError::IntentNotFound(intent_id.clone()))?;
        let ctx = ExecContext {
            spec,
            scope_id: root_scope,
            run_id: RunId::new(),
            generation,
            snapshot_id: None,
        };
        let lease = self.admit_external_action(&ctx)?;
        let tool_call_id = ToolCallId::new();
        self.record_action_state(
            intent_id,
            &action,
            ConnectorActionStatus::Compensating,
            action.attention_id.clone(),
            Some(tool_call_id.clone()),
            None,
            None,
        )?;
        let executor = AdapterActionExecutor {
            adapter,
            connector: connector.clone(),
            credential,
            mode: ActionMode::Compensate,
        };
        let tools = ToolGateway::new(&self.ledger, self.workspace.clone())
            .with_external_executor(&executor);
        let compensation_key = format!("compensate:{}", action.idempotency_key);
        if tools.effect_is_authorized(&ctx, action.effect, &compensation_key)? {
            self.record_action_state(
                intent_id,
                &action,
                ConnectorActionStatus::CompensationExecuting,
                action.attention_id.clone(),
                Some(tool_call_id.clone()),
                None,
                None,
            )?;
        }
        let mut compensation_input = action.input.clone();
        if let Value::Object(object) = &mut compensation_input {
            object.insert(
                "__else_action".into(),
                serde_json::to_value(&action).map_err(|error| {
                    ApiError::Conflict(format!("serialize compensation action: {error}"))
                })?,
            );
        }
        let call = ToolCall {
            id: tool_call_id.clone(),
            tool: format!("connector:{}:{}", connector.id, action.capability),
            effect: action.effect,
            input: compensation_input,
            idempotency_key: compensation_key,
        };
        let result = tools.execute(&ctx, call);
        let _ = self.revoke_lease(&ctx, &lease);
        match result {
            Ok(outcome) => {
                let status = status_from_outcome(&outcome, ActionMode::Compensate);
                self.record_action_state(
                    intent_id,
                    &action,
                    status,
                    action.attention_id.clone(),
                    Some(tool_call_id),
                    Some(outcome),
                    None,
                )?;
                self.projected_action(intent_id, action_id)
            }
            Err(GatewayError::NeedsApproval(request_id)) => {
                let pending_request = request_id.clone();
                self.record_action_state(
                    intent_id,
                    &action,
                    ConnectorActionStatus::Compensating,
                    Some(request_id),
                    Some(tool_call_id),
                    None,
                    None,
                )?;
                Err(GatewayError::NeedsApproval(pending_request).into())
            }
            Err(error) => {
                self.record_action_state(
                    intent_id,
                    &action,
                    ConnectorActionStatus::CompensationFailed,
                    action.attention_id.clone(),
                    Some(tool_call_id),
                    None,
                    Some(error.to_string()),
                )?;
                Err(error.into())
            }
        }
    }

    fn projected_action(
        &self,
        intent_id: &IntentId,
        action_id: &ConnectorActionId,
    ) -> Result<ConnectorAction> {
        self.projection(intent_id)?
            .connector_actions
            .get(action_id)
            .cloned()
            .ok_or_else(|| ApiError::NotFound(format!("connector action {action_id}")))
    }

    #[allow(clippy::too_many_arguments)]
    fn record_action_state(
        &self,
        intent_id: &IntentId,
        action: &ConnectorAction,
        status: ConnectorActionStatus,
        attention_id: Option<else_protocol::ids::AttentionId>,
        tool_call_id: Option<ToolCallId>,
        outcome: Option<ToolOutcome>,
        error: Option<String>,
    ) -> Result<()> {
        let (root, generation, revision) = self.kernel_context(intent_id)?;
        self.ledger.append(&EventEnvelope::new(
            intent_id.clone(),
            root,
            generation,
            revision,
            Actor::System,
            EventPayload::ConnectorActionStateChanged {
                action_id: action.id.clone(),
                status,
                attention_id,
                tool_call_id,
                outcome,
                error,
            },
        ))?;
        Ok(())
    }
}

struct AdapterActionExecutor<'a> {
    adapter: &'a dyn else_connector::ConnectorAdapter,
    connector: ConnectorRegistration,
    credential: Option<ConnectorCredential>,
    mode: ActionMode,
}

impl ExternalActionExecutor for AdapterActionExecutor<'_> {
    fn execute_external(&self, binding: &ExternalActionBinding, input: &Value) -> ToolOutcome {
        if binding.connector_id != self.connector.id {
            return ToolOutcome {
                status: ToolStatus::Failed,
                output: json!({"error": "external action binding connector mismatch"}),
                effect: EffectConclusion::NoEffect,
            };
        }
        let result = match self.mode {
            ActionMode::Execute => self.adapter.execute_action(
                &self.connector.id,
                &self.connector.source,
                &binding.operation,
                self.credential.as_ref(),
                input,
            ),
            ActionMode::Compensate => {
                let original = input
                    .get("__else_action")
                    .cloned()
                    .and_then(|value| serde_json::from_value::<ConnectorAction>(value).ok());
                match original {
                    Some(action) => self.adapter.compensate_action(
                        &self.connector.source,
                        &action,
                        self.credential.as_ref(),
                    ),
                    None => Err(else_connector::ConnectorError::ActionUnsupported {
                        adapter: self.connector.adapter.clone(),
                        capability: format!("{}#compensate-context", binding.operation),
                    }),
                }
            }
        };
        result.unwrap_or_else(|error| ToolOutcome {
            status: ToolStatus::Failed,
            output: json!({"error": error.to_string()}),
            effect: EffectConclusion::NoEffect,
        })
    }
}

#[derive(Clone, Copy)]
enum ActionMode {
    Execute,
    Compensate,
}

fn status_from_outcome(outcome: &ToolOutcome, mode: ActionMode) -> ConnectorActionStatus {
    match mode {
        ActionMode::Compensate
            if outcome.status == ToolStatus::Succeeded
                && outcome.effect == EffectConclusion::Compensated =>
        {
            ConnectorActionStatus::Compensated
        }
        ActionMode::Compensate if outcome.status == ToolStatus::Failed => {
            ConnectorActionStatus::CompensationFailed
        }
        ActionMode::Compensate if outcome.status == ToolStatus::Unknown => {
            ConnectorActionStatus::Unknown
        }
        _ => match outcome.status {
            ToolStatus::Succeeded => ConnectorActionStatus::Applied,
            ToolStatus::Failed => ConnectorActionStatus::Failed,
            ToolStatus::Unknown => ConnectorActionStatus::Unknown,
        },
    }
}
