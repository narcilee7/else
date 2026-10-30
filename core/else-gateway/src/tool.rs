//! Tool Gateway：所有副作用必须经过这里。
//!
//! 职责（W2 薄版）：副作用声明、授权检查、幂等、事件落账。
//! 授权规则：副作用等级超过 IntentSpec 授予的自主上限时，
//! 不执行，而是提交 AttentionRequest 并返回 NeedsApproval。

use std::path::{Component, Path, PathBuf};

use else_attention::AttentionKernel;
use else_ledger::Ledger;
use else_protocol::attention::{AttentionKind, AttentionRequest, DecisionOption, ExpectedLoss};
use else_protocol::effect::{EffectConclusion, SideEffectKind, ToolCall, ToolOutcome, ToolStatus};
use else_protocol::event::{Actor, EventPayload};
use else_protocol::ids::AttentionId;
use else_protocol::intent::IntentState;
use serde_json::{json, Value};

use crate::resource::{ensure_active_lease, ensure_tool_capacity};
use crate::{ensure_not_fenced, ExecContext, GatewayError, Result};

pub struct ToolGateway<'a> {
    ledger: &'a Ledger,
    workspace: PathBuf,
}

impl<'a> ToolGateway<'a> {
    pub fn new(ledger: &'a Ledger, workspace: PathBuf) -> Self {
        Self { ledger, workspace }
    }

    pub fn workspace(&self) -> &Path {
        &self.workspace
    }

    /// 执行一次工具调用：围栏 → 幂等 → 落账 → 授权 → 执行 → 围栏 → 落账。
    pub fn execute(&self, ctx: &ExecContext, call: ToolCall) -> Result<ToolOutcome> {
        ensure_not_fenced(self.ledger, ctx)?;
        // 幂等：相同 idempotency_key 已完成的调用不重复执行（防重试/晚到回调）。
        if let Some(outcome) = self.find_completed(ctx, &call.idempotency_key)? {
            return Ok(outcome);
        }
        ensure_active_lease(self.ledger, ctx)?;
        ensure_tool_capacity(self.ledger, ctx)?;

        ctx.record(
            self.ledger,
            Actor::Agent,
            EventPayload::ToolCalled { call: call.clone() },
        )?;

        self.check_authority(ctx, &call)?;
        ensure_not_fenced(self.ledger, ctx)?;
        let outcome = self.run_tool(&call);
        self.complete(ctx, call.id, outcome)
    }

    /// 工具完成回调落账。Abort 后的晚到回调在这里被围栏，不能覆盖收尾结论。
    pub fn complete(
        &self,
        ctx: &ExecContext,
        call_id: else_protocol::ids::ToolCallId,
        outcome: ToolOutcome,
    ) -> Result<ToolOutcome> {
        ensure_not_fenced(self.ledger, ctx)?;
        ensure_active_lease(self.ledger, ctx)?;
        ctx.record(
            self.ledger,
            Actor::Tool,
            EventPayload::ToolCompleted {
                call_id,
                outcome: outcome.clone(),
            },
        )?;
        Ok(outcome)
    }

    /// 副作用超过自主上限 → 提交 AttentionRequest，等待本人。
    fn check_authority(&self, ctx: &ExecContext, call: &ToolCall) -> Result<()> {
        if call.effect <= ctx.spec.authority.max_autonomous_effect {
            return Ok(());
        }
        // 一次性授权：本人已批准过同一幂等键的调用 → 放行，不扩大全局权限。
        if self.approved(ctx, &call.idempotency_key)? {
            return Ok(());
        }
        let request = AttentionRequest {
            id: AttentionId::new(),
            kind: AttentionKind::Authorization,
            summary: format!("是否批准执行 `{}`？", call.tool),
            blocked_reason: format!(
                "副作用等级 {:?} 超出当前授权上限 {:?}",
                call.effect, ctx.spec.authority.max_autonomous_effect
            ),
            expected_loss: ExpectedLoss::High,
            blocking: true,
            reversible: call.effect <= SideEffectKind::Compensatable,
            decide_by: None,
            options: vec![
                DecisionOption {
                    id: "approve".into(),
                    label: "批准本次执行".into(),
                    recommended: false,
                },
                DecisionOption {
                    id: "reject".into(),
                    label: "拒绝，换一条路径".into(),
                    recommended: true,
                },
            ],
            default_option: Some("reject".into()),
            related_call_key: Some(call.idempotency_key.clone()),
        };
        let request_id = request.id.clone();
        let ruling = AttentionKernel.rule(&request, &ctx.spec.attention_policy, chrono::Utc::now());
        ctx.record(
            self.ledger,
            Actor::System,
            EventPayload::AttentionRequested { request },
        )?;
        ctx.record(
            self.ledger,
            Actor::System,
            EventPayload::AttentionRuled {
                request_id: request_id.clone(),
                ruling,
            },
        )?;
        ctx.record(
            self.ledger,
            Actor::System,
            EventPayload::IntentStateChanged {
                state: IntentState::NeedsDecision,
            },
        )?;
        Err(GatewayError::NeedsApproval(request_id))
    }

    /// 一次性授权查找：同一幂等键的调用是否已被本人批准。
    fn approved(&self, ctx: &ExecContext, key: &str) -> Result<bool> {
        let events = self.ledger.read_intent(&ctx.spec.id)?;
        let mut request_keys = std::collections::HashMap::new();
        for event in events {
            // 授权按 Generation 隔离：旧世代的批准不泄漏到分支
            if event.envelope.generation != ctx.generation
                || event.envelope.scope_id != ctx.scope_id
            {
                continue;
            }
            match event.envelope.payload {
                EventPayload::AttentionRequested { request } => {
                    if let Some(call_key) = request.related_call_key {
                        request_keys.insert(request.id, call_key);
                    }
                }
                EventPayload::DecisionResolved { decision } => {
                    let approved = matches!(
                        decision.choice,
                        else_protocol::attention::DecisionChoice::Option(ref id) if id == "approve"
                    );
                    if approved
                        && request_keys.get(&decision.request_id).map(String::as_str) == Some(key)
                    {
                        return Ok(true);
                    }
                }
                _ => {}
            }
        }
        Ok(false)
    }

    /// 幂等查找：同一 Intent、同一 Generation 内相同幂等键的已完成调用。
    fn find_completed(&self, ctx: &ExecContext, key: &str) -> Result<Option<ToolOutcome>> {
        let events = self.ledger.read_intent(&ctx.spec.id)?;
        let mut call_keys = std::collections::HashMap::new();
        let mut completed = std::collections::HashMap::new();
        for event in events {
            // 分支隔离：其他 Generation 的完成结果不命中本分支的幂等
            if event.envelope.generation != ctx.generation
                || event.envelope.scope_id != ctx.scope_id
            {
                continue;
            }
            match event.envelope.payload {
                EventPayload::ToolCalled { call } => {
                    call_keys.insert(call.id, call.idempotency_key);
                }
                EventPayload::ToolCompleted { call_id, outcome } => {
                    completed.insert(call_id, outcome);
                }
                _ => {}
            }
        }
        for (call_id, outcome) in completed {
            if call_keys.get(&call_id).map(String::as_str) == Some(key) {
                return Ok(Some(outcome));
            }
        }
        Ok(None)
    }

    /// 实际执行。工具失败是正常结果（Failed outcome），不是系统错误。
    fn run_tool(&self, call: &ToolCall) -> ToolOutcome {
        let result = match call.tool.as_str() {
            "fs.read" => self.fs_read(&call.input),
            "fs.write" => self.fs_write(&call.input),
            "shell" => self.shell(&call.input),
            other => {
                return failed(json!({ "error": format!("unknown tool: {other}") }));
            }
        };
        match result {
            Ok((output, effect)) => ToolOutcome {
                status: ToolStatus::Succeeded,
                output,
                effect,
            },
            Err(output) => failed(output),
        }
    }

    fn fs_read(&self, input: &Value) -> std::result::Result<(Value, EffectConclusion), Value> {
        let path = self.resolve(input)?;
        let content = std::fs::read_to_string(&path)
            .map_err(|e| json!({ "error": format!("read {}: {e}", path.display()) }))?;
        Ok((json!({ "content": content }), EffectConclusion::NoEffect))
    }

    fn fs_write(&self, input: &Value) -> std::result::Result<(Value, EffectConclusion), Value> {
        let path = self.resolve(input)?;
        let content = input["content"].as_str().unwrap_or_default();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| json!({ "error": e.to_string() }))?;
        }
        std::fs::write(&path, content)
            .map_err(|e| json!({ "error": format!("write {}: {e}", path.display()) }))?;
        Ok((
            json!({ "path": path.display().to_string(), "bytes": content.len() }),
            EffectConclusion::Applied,
        ))
    }

    fn shell(&self, input: &Value) -> std::result::Result<(Value, EffectConclusion), Value> {
        let command = input["command"].as_str().unwrap_or_default();
        let sandbox =
            else_environment::Sandbox::new(else_environment::SandboxProfile::new(&self.workspace))
                .map_err(|e| json!({ "error": e.to_string() }))?;
        let backend = sandbox.backend().name();
        let output = sandbox
            .run_shell(command)
            .map_err(|e| json!({ "error": e.to_string() }))?;
        Ok((
            json!({
                "code": output.status.code(),
                "stdout": String::from_utf8_lossy(&output.stdout),
                "stderr": String::from_utf8_lossy(&output.stderr),
                "sandbox": backend,
            }),
            EffectConclusion::Applied,
        ))
    }

    /// 路径必须留在工作区内：拒绝绝对路径、`..` 与符号链接逃逸。
    /// 对尚不存在的写目标，从最近的现存祖先解析并重新拼接。
    fn resolve(&self, input: &Value) -> std::result::Result<PathBuf, Value> {
        let raw = input["path"].as_str().unwrap_or_default();
        let path = Path::new(raw);
        let invalid = raw.is_empty()
            || path.is_absolute()
            || path.components().any(|c| {
                matches!(
                    c,
                    Component::ParentDir | Component::RootDir | Component::Prefix(_)
                )
            });
        if invalid {
            return Err(json!({ "error": format!("path escapes workspace: {raw}") }));
        }
        let workspace = self
            .workspace
            .canonicalize()
            .map_err(|e| json!({ "error": format!("resolve workspace: {e}") }))?;
        let candidate = workspace.join(path);
        let mut existing = candidate.as_path();
        loop {
            match std::fs::symlink_metadata(existing) {
                Ok(_) => break,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    existing = existing.parent().ok_or_else(
                        || json!({ "error": format!("path has no existing ancestor: {raw}") }),
                    )?;
                }
                Err(error) => return Err(json!({ "error": error.to_string() })),
            }
        }
        let canonical = existing
            .canonicalize()
            .map_err(|e| json!({ "error": format!("resolve {raw}: {e}") }))?;
        if !canonical.starts_with(&workspace) {
            return Err(json!({ "error": format!("path escapes workspace: {raw}") }));
        }
        let suffix = candidate
            .strip_prefix(existing)
            .map_err(|e| json!({ "error": e.to_string() }))?;
        if suffix.as_os_str().is_empty() {
            Ok(canonical)
        } else {
            Ok(canonical.join(suffix))
        }
    }
}

fn failed(output: Value) -> ToolOutcome {
    ToolOutcome {
        status: ToolStatus::Failed,
        output,
        effect: EffectConclusion::Unknown,
    }
}
