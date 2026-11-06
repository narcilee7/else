//! Runtime HTTP 客户端。
//!
//! 每个方法对应 daemon 的一条路由，返回值直接是契约类型
//! （[`else_protocol::api`] 里的形状或协议类型本身）。客户端不做业务判断：
//! 编排、排序与解释属于上层 surface。

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use else_protocol::api::{
    AttentionView, AttentionWindowSnapshot, DriveView, EventResponse, HarnessView, MemoryView,
    PlanView, ResourceView, ScopeView, ThreadSnapshot,
};
use else_protocol::artifact::Checkpoint;
use else_protocol::attention::{AttentionRequest, DecisionChoice};
use else_protocol::capability::{AgentBinding, CapabilityRegistration};
use else_protocol::connector::{
    ConnectorAction, ConnectorCredential, ConnectorCredentialMetadata, ConnectorHealthReport,
    ConnectorManifest, ConnectorRegistration, ConnectorRepairReport, ConnectorSyncResult,
    ThreadCandidate, ThreadCandidateRecord, WorldSignal, WorldSignalRecord,
};
use else_protocol::effect::SideEffectKind;
use else_protocol::ids::{
    AttentionId, ConnectorActionId, ConnectorId, MemoryId, ScopeId, ThreadCandidateId, ThreadId,
    WorldItemId,
};
use else_protocol::memory::{MemoryRecord, MemoryScope};
use else_protocol::plan::PlanImpact;
use else_protocol::runtime::RuntimeSettings;
use else_protocol::thread::{OpaqueHarnessPolicy, ThreadSpec};
use else_protocol::world::{
    IdentityWorld, PersonalBrief, ProjectWorld, WorldCorrelationCorrection, WorldItem,
    WorldItemResolutionKind,
};
use serde::de::DeserializeOwned;
use serde_json::{json, Value};

use crate::{ClientError, Result};

/// Runtime 的契约版本。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct RuntimeVersion {
    /// HTTP 契约版本。
    pub contract_version: String,
    /// 事件协议版本。
    pub protocol_version: String,
}

/// 一次异步操作的状态。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct RuntimeOperation {
    pub id: String,
    pub thread_id: String,
    pub kind: String,
    /// `running` / `completed` / `failed`。
    pub status: String,
    #[serde(default)]
    pub error: Option<String>,
    /// 操作产出的结果；drive 带上停止原因与步数。
    #[serde(default)]
    pub result: Option<Value>,
}

impl RuntimeOperation {
    pub fn is_finished(&self) -> bool {
        self.status != "running"
    }
}

/// Runtime 客户端。
///
/// 线程安全：可以放在 TUI 的后台刷新线程里用。
#[derive(Clone)]
pub struct RuntimeClient {
    base: String,
    token: String,
    agent: ureq::Agent,
    /// 每个 Thread 的当前 Revision。写操作用它做 `If-Match` 前置条件。
    ///
    /// 共享的：客户端被克隆给多个线程时，Revision 缓存也只有一份，
    /// 不会出现两个线程各写各的 Revision。
    revisions: Arc<Mutex<HashMap<String, u64>>>,
}

impl RuntimeClient {
    /// 用地址与令牌构造客户端。
    pub fn new(base: &str, token: &str) -> Self {
        Self {
            base: base.trim_end_matches('/').to_string(),
            token: token.to_string(),
            agent: ureq::AgentBuilder::new()
                .timeout(std::time::Duration::from_secs(30))
                .build(),
            revisions: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Runtime 地址。
    pub fn base_url(&self) -> &str {
        &self.base
    }

    // ——————————————————————————— 传输原语 ———————————————————————————

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.base, path)
    }

    fn authenticated(&self, request: ureq::Request) -> ureq::Request {
        request.set("Authorization", &format!("Bearer {}", self.token))
    }

    fn send(&self, outcome: std::result::Result<ureq::Response, ureq::Error>) -> Result<Value> {
        let response = outcome.map_err(|error| match error {
            ureq::Error::Status(code, response) => {
                let body = response.into_string().unwrap_or_default();
                ClientError::Rejected(format!("{code}: {}", body.trim()))
            }
            ureq::Error::Transport(transport) => ClientError::Unreachable(transport.to_string()),
        })?;
        response
            .into_json()
            .map_err(|error| ClientError::Malformed(error.to_string()))
    }

    fn get_value(&self, path: &str) -> Result<Value> {
        self.send(self.authenticated(self.agent.get(&self.url(path))).call())
    }

    /// 发送一次写请求。
    ///
    /// `idempotency` 是必填的：daemon 用它去重，重试不会重复执行副作用。
    fn post_value(
        &self,
        path: &str,
        body: &Value,
        revision: Option<u64>,
        idempotency: &str,
    ) -> Result<Value> {
        let mut request = self.authenticated(self.agent.post(&self.url(path)));
        if let Some(revision) = revision {
            request = request.set("If-Match", &revision.to_string());
        }
        request = request.set("Idempotency-Key", idempotency);
        self.send(request.send_json(body.clone()))
    }

    fn get_typed<T: DeserializeOwned>(&self, path: &str) -> Result<T> {
        let value = self.get_value(path)?;
        serde_json::from_value(value).map_err(|error| ClientError::Malformed(error.to_string()))
    }

    fn post_typed<T: DeserializeOwned>(
        &self,
        path: &str,
        body: &Value,
        revision: Option<u64>,
        idempotency: &str,
    ) -> Result<T> {
        let value = self.post_value(path, body, revision, idempotency)?;
        serde_json::from_value(value).map_err(|error| ClientError::Malformed(error.to_string()))
    }

    // ——————————————————————————— Revision ———————————————————————————

    fn remember_revision(&self, thread_id: &ThreadId, revision: u64) {
        self.revisions
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .insert(thread_id.to_string(), revision);
    }

    fn cached_revision(&self, thread_id: &ThreadId) -> Option<u64> {
        self.revisions
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .get(&thread_id.to_string())
            .copied()
    }

    /// 等一个异步操作结束，然后重新读 Thread 快照。
    ///
    /// 写操作在 daemon 侧是异步的（202 + operation_id）。这里等到它结束再读，
    /// surface 拿到的就是**已经发生**的状态，而不是发起时的状态。
    fn settle(&self, value: Value, thread_id: &ThreadId) -> Result<ThreadSnapshot> {
        let snapshot = self.snapshot_of(value)?;
        if let Some(operation_id) = snapshot.operation_id.clone() {
            self.wait_for_operation(&operation_id, Duration::from_secs(600))?;
        }
        self.thread(thread_id)
    }

    /// 轮询一个异步操作直到结束。
    pub fn wait_for_operation(
        &self,
        operation_id: &str,
        timeout: Duration,
    ) -> Result<RuntimeOperation> {
        let deadline = std::time::Instant::now() + timeout;
        loop {
            let operation = self.operation(operation_id)?;
            if operation.is_finished() {
                if operation.status == "failed" {
                    return Err(ClientError::Rejected(
                        operation
                            .error
                            .unwrap_or_else(|| format!("operation {operation_id} failed")),
                    ));
                }
                return Ok(operation);
            }
            if std::time::Instant::now() >= deadline {
                return Err(ClientError::Unreachable(format!(
                    "operation {operation_id} did not finish within {timeout:?}"
                )));
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    /// 取写操作需要的 Revision。
    ///
    /// 缓存里没有就先读一次 Thread —— 读到的 Revision 立刻回填缓存。
    fn revision_for(&self, thread_id: &ThreadId) -> Result<u64> {
        if let Some(revision) = self.cached_revision(thread_id) {
            return Ok(revision);
        }
        Ok(self.thread(thread_id)?.spec.revision)
    }

    /// 把一份快照读成契约类型，并顺手记住它的 Revision。
    fn snapshot_of(&self, value: Value) -> Result<ThreadSnapshot> {
        let snapshot: ThreadSnapshot = serde_json::from_value(value)
            .map_err(|error| ClientError::Malformed(error.to_string()))?;
        self.remember_revision(&snapshot.spec.id, snapshot.spec.revision);
        Ok(snapshot)
    }

    // ——————————————————————————— Thread 生命周期 ———————————————————————————

    /// 列出全部 Thread。
    pub fn list_threads(&self) -> Result<Vec<ThreadSnapshot>> {
        let value = self.get_value("/v1/threads")?;
        let snapshots: Vec<ThreadSnapshot> = serde_json::from_value(value)
            .map_err(|error| ClientError::Malformed(error.to_string()))?;
        for snapshot in &snapshots {
            self.remember_revision(&snapshot.spec.id, snapshot.spec.revision);
        }
        Ok(snapshots)
    }

    /// 读一个 Thread 的快照。
    pub fn thread(&self, thread_id: &ThreadId) -> Result<ThreadSnapshot> {
        let value = self.get_value(&format!("/v1/threads/{thread_id}"))?;
        self.snapshot_of(value)
    }

    /// 创建一个 Thread。
    pub fn create_thread(&self, outcome: &str, project: Option<&str>) -> Result<ThreadSnapshot> {
        let body = json!({ "outcome": outcome, "project": project });
        let key = unique_key("create-thread");
        let value = self.post_value("/v1/threads", &body, None, &key)?;
        self.snapshot_of(value)
    }

    /// 推进一个 Kernel 步。
    pub fn advance(&self, thread_id: &ThreadId, harness: Option<&str>) -> Result<ThreadSnapshot> {
        let revision = self.revision_for(thread_id)?;
        let body = json!({ "harness": harness });
        let key = operation_key(&["advance", &thread_id.to_string(), &revision.to_string()]);
        let value = self.post_value(
            &format!("/v1/threads/{thread_id}/advance"),
            &body,
            Some(revision),
            &key,
        )?;
        self.settle(value, thread_id)
    }

    /// 连续推进，直到等待用户、Run、Timer 或空闲。
    pub fn drive(
        &self,
        thread_id: &ThreadId,
        harness: Option<&str>,
        max_steps: u64,
    ) -> Result<DriveView> {
        let revision = self.revision_for(thread_id)?;
        let body = json!({ "harness": harness, "max_steps": max_steps });
        let key = operation_key(&[
            "drive",
            &thread_id.to_string(),
            &revision.to_string(),
            &max_steps.to_string(),
        ]);
        let value = self.post_value(
            &format!("/v1/threads/{thread_id}/drive"),
            &body,
            Some(revision),
            &key,
        )?;
        let snapshot = self.snapshot_of(value)?;
        let mut stopped = String::new();
        let mut steps = 0;
        if let Some(operation_id) = snapshot.operation_id.clone() {
            let operation = self.wait_for_operation(&operation_id, Duration::from_secs(600))?;
            if let Some(result) = operation.result {
                stopped = result["stopped"].as_str().unwrap_or_default().to_string();
                steps = result["steps"].as_u64().unwrap_or_default();
            }
        }
        Ok(DriveView {
            snapshot: self.thread(thread_id)?,
            stopped,
            steps,
        })
    }

    /// 启动一次执行。
    pub fn run(&self, thread_id: &ThreadId, harness: Option<&str>) -> Result<ThreadSnapshot> {
        let revision = self.revision_for(thread_id)?;
        let body = json!({ "harness": harness });
        let key = operation_key(&["run", &thread_id.to_string(), &revision.to_string()]);
        let value = self.post_value(
            &format!("/v1/threads/{thread_id}/run"),
            &body,
            Some(revision),
            &key,
        )?;
        self.settle(value, thread_id)
    }

    /// 取消执行并收敛在途副作用。
    pub fn abort(&self, thread_id: &ThreadId, reason: &str) -> Result<ThreadSnapshot> {
        let revision = self.revision_for(thread_id)?;
        let body = json!({ "reason": reason });
        let key = operation_key(&["abort", &thread_id.to_string(), &revision.to_string()]);
        let value = self.post_value(
            &format!("/v1/threads/{thread_id}/abort"),
            &body,
            Some(revision),
            &key,
        )?;
        self.snapshot_of(value)
    }

    /// 修订 Thread 目标。
    pub fn revise_thread(
        &self,
        thread_id: &ThreadId,
        outcome: Option<&str>,
        add_constraints: &[String],
        reason: &str,
    ) -> Result<ThreadSpec> {
        let revision = self.revision_for(thread_id)?;
        let body = json!({
            "outcome": outcome,
            "add_constraints": add_constraints,
            "reason": reason,
        });
        let key = operation_key(&["revise", &thread_id.to_string(), &revision.to_string()]);
        let spec: ThreadSpec = self.post_typed(
            &format!("/v1/threads/{thread_id}/revise"),
            &body,
            Some(revision),
            &key,
        )?;
        self.remember_revision(thread_id, spec.revision);
        Ok(spec)
    }

    /// 删除一个 Thread 及其关联数据。
    pub fn delete_thread(&self, thread_id: &ThreadId) -> Result<Value> {
        let revision = self.revision_for(thread_id)?;
        let request = self
            .authenticated(
                self.agent
                    .delete(&self.url(&format!("/v1/threads/{thread_id}"))),
            )
            .set("If-Match", &revision.to_string())
            .set("Idempotency-Key", &format!("delete-{thread_id}"));
        self.send(request.call())
    }

    // ——————————————————————————— Scope ———————————————————————————

    /// 读 Scope 委托树。
    pub fn scopes(&self, thread_id: &ThreadId) -> Result<Vec<ScopeView>> {
        self.get_typed(&format!("/v1/threads/{thread_id}/scopes"))
    }

    /// 委托一个子 Scope。
    pub fn delegate(
        &self,
        thread_id: &ThreadId,
        parent_scope: Option<&ScopeId>,
        task: &str,
        depends_on: &[ScopeId],
        max_effect: SideEffectKind,
    ) -> Result<ScopeId> {
        let revision = self.revision_for(thread_id)?;
        let body = json!({
            "parent_scope": parent_scope.map(|scope| scope.to_string()),
            "task": task,
            "depends_on": depends_on.iter().map(|scope| scope.to_string()).collect::<Vec<_>>(),
            "max_effect": side_effect_name(max_effect),
        });
        let key = operation_key(&["scope", &thread_id.to_string(), &revision.to_string(), task]);
        let value: Value = self.post_typed(
            &format!("/v1/threads/{thread_id}/scopes"),
            &body,
            Some(revision),
            &key,
        )?;
        Ok(ScopeId::from(
            value["scope_id"].as_str().unwrap_or_default().to_string(),
        ))
    }

    /// 启动或续跑一个 Scope。
    pub fn run_scope(
        &self,
        thread_id: &ThreadId,
        scope_id: &ScopeId,
        harness: Option<&str>,
    ) -> Result<ThreadSnapshot> {
        let revision = self.revision_for(thread_id)?;
        let body = json!({ "harness": harness });
        let key = operation_key(&[
            "run-scope",
            &thread_id.to_string(),
            &scope_id.to_string(),
            &revision.to_string(),
        ]);
        let value = self.post_value(
            &format!("/v1/threads/{thread_id}/scopes/{scope_id}/run"),
            &body,
            Some(revision),
            &key,
        )?;
        self.settle(value, thread_id)
    }

    // ——————————————————————————— 注意力与决策 ———————————————————————————

    /// 全部注意力窗口。
    pub fn attention_windows(&self) -> Result<Vec<AttentionWindowSnapshot>> {
        self.get_typed("/v1/attention")
    }

    /// 一个 Thread 的注意力视图：打断成本、全部窗口与待决请求。
    pub fn attention(&self, thread_id: &ThreadId) -> Result<AttentionView> {
        self.get_typed(&format!("/v1/threads/{thread_id}/attention"))
    }

    /// 提交一个决策。
    pub fn decide(
        &self,
        thread_id: &ThreadId,
        request_id: &AttentionId,
        choice: DecisionChoice,
    ) -> Result<ThreadSnapshot> {
        let revision = self.revision_for(thread_id)?;
        let body = json!({ "choice": choice });
        let key = operation_key(&[
            "decide",
            &thread_id.to_string(),
            &request_id.to_string(),
            &revision.to_string(),
        ]);
        let value = self.post_value(
            &format!("/v1/threads/{thread_id}/decisions/{request_id}/resolve"),
            &body,
            Some(revision),
            &key,
        )?;
        self.settle(value, thread_id)
    }

    /// 提交一条注意力请求。
    pub fn submit_attention_request(
        &self,
        thread_id: &ThreadId,
        scope_id: Option<&ScopeId>,
        request: AttentionRequest,
    ) -> Result<AttentionId> {
        let revision = self.revision_for(thread_id)?;
        let body = json!({
            "scope_id": scope_id.map(|scope| scope.to_string()),
            "request": request,
        });
        let key = operation_key(&[
            "attention",
            &thread_id.to_string(),
            &revision.to_string(),
            &request.id.to_string(),
        ]);
        let value: Value = self.post_typed(
            &format!("/v1/threads/{thread_id}/attention"),
            &body,
            Some(revision),
            &key,
        )?;
        Ok(AttentionId::from(
            value["id"].as_str().unwrap_or_default().to_string(),
        ))
    }

    // ——————————————————————————— 唤醒 ———————————————————————————

    /// 标记 Thread 正在等待外部事实。
    pub fn wait_for_external(&self, thread_id: &ThreadId, reason: &str) -> Result<ThreadSnapshot> {
        let revision = self.revision_for(thread_id)?;
        let body = json!({ "reason": reason });
        let key = operation_key(&[
            "wait-external",
            &thread_id.to_string(),
            &revision.to_string(),
        ]);
        let value = self.post_value(
            &format!("/v1/threads/{thread_id}/wait-external"),
            &body,
            Some(revision),
            &key,
        )?;
        self.snapshot_of(value)
    }

    /// 记录一条外部事件并唤醒 Thread。
    pub fn record_external_event(
        &self,
        thread_id: &ThreadId,
        source: &str,
        summary: &str,
    ) -> Result<ThreadSnapshot> {
        let revision = self.revision_for(thread_id)?;
        let body = json!({ "source": source, "summary": summary });
        let key = operation_key(&[
            "event",
            &thread_id.to_string(),
            &revision.to_string(),
            source,
            summary,
        ]);
        let value = self.post_value(
            &format!("/v1/threads/{thread_id}/events"),
            &body,
            Some(revision),
            &key,
        )?;
        self.snapshot_of(value)
    }

    /// 安排一个持久化唤醒 Timer。
    pub fn schedule_timer(
        &self,
        thread_id: &ThreadId,
        seconds: i64,
        reason: &str,
    ) -> Result<String> {
        let revision = self.revision_for(thread_id)?;
        let body = json!({ "seconds": seconds, "reason": reason });
        let key = operation_key(&[
            "timer",
            &thread_id.to_string(),
            &revision.to_string(),
            &seconds.to_string(),
        ]);
        let value: Value = self.post_typed(
            &format!("/v1/threads/{thread_id}/timers"),
            &body,
            Some(revision),
            &key,
        )?;
        Ok(value["timer_id"].as_str().unwrap_or_default().to_string())
    }

    // ——————————————————————————— 计划 ———————————————————————————

    /// 当前计划、版本历史与影响分析。
    pub fn plan(&self, thread_id: &ThreadId) -> Result<PlanView> {
        self.get_typed(&format!("/v1/threads/{thread_id}/plan"))
    }

    /// 分析一条外部事实对当前计划的影响。
    pub fn analyze_plan_impact(
        &self,
        thread_id: &ThreadId,
        source_event_id: &str,
        source: &str,
        summary: &str,
    ) -> Result<Option<PlanImpact>> {
        let revision = self.revision_for(thread_id)?;
        let body = json!({
            "source_event_id": source_event_id,
            "source": source,
            "summary": summary,
        });
        let key = operation_key(&["plan-impact", &thread_id.to_string(), &revision.to_string()]);
        self.post_typed(
            &format!("/v1/threads/{thread_id}/plan/impact"),
            &body,
            Some(revision),
            &key,
        )
    }

    // ——————————————————————————— 记忆 ———————————————————————————

    /// 记忆投影。
    pub fn memory(&self) -> Result<MemoryView> {
        self.get_typed("/v1/threads/else-system/memory")
    }

    /// 晋升一条记忆候选。
    pub fn promote_memory(
        &self,
        memory_id: &MemoryId,
        scope: Option<&MemoryScope>,
    ) -> Result<MemoryRecord> {
        let body = json!({
            "candidate_id": memory_id.to_string(),
            "scope": scope.map(memory_scope_name),
        });
        let key = operation_key(&["memory-promote", &memory_id.to_string()]);
        self.post_typed("/v1/threads/else-system/memory/promote", &body, None, &key)
    }

    /// 撤销一条记忆。
    pub fn revoke_memory(&self, memory_id: &MemoryId, reason: &str) -> Result<()> {
        let body = json!({ "memory_id": memory_id.to_string(), "reason": reason });
        let key = operation_key(&["memory-revoke", &memory_id.to_string()]);
        let _: Value =
            self.post_typed("/v1/threads/else-system/memory/revoke", &body, None, &key)?;
        Ok(())
    }

    /// 记录一条项目级用户纠正。
    pub fn correct_memory(
        &self,
        project: &str,
        subject: &str,
        predicate: &str,
        value: &str,
    ) -> Result<MemoryRecord> {
        let body = json!({
            "project": project,
            "subject": subject,
            "predicate": predicate,
            "value": value,
        });
        let key = operation_key(&["memory-correct", project, subject, predicate, value]);
        self.post_typed("/v1/threads/else-system/memory/correct", &body, None, &key)
    }

    // ——————————————————————————— 执行与工件 ———————————————————————————

    /// 创建一个检查点。
    pub fn checkpoint(&self, thread_id: &ThreadId) -> Result<Checkpoint> {
        let revision = self.revision_for(thread_id)?;
        let key = operation_key(&["checkpoint", &thread_id.to_string(), &revision.to_string()]);
        self.post_typed(
            &format!("/v1/threads/{thread_id}/checkpoints"),
            &json!({}),
            Some(revision),
            &key,
        )
    }

    /// 从检查点重跑一个 Scope。
    pub fn rerun(
        &self,
        thread_id: &ThreadId,
        checkpoint_id: &str,
        alternative: bool,
    ) -> Result<(ScopeId, u64)> {
        let revision = self.revision_for(thread_id)?;
        let body = json!({ "checkpoint_id": checkpoint_id, "alternative": alternative });
        let key = operation_key(&[
            "rerun",
            &thread_id.to_string(),
            checkpoint_id,
            &alternative.to_string(),
        ]);
        let value: Value = self.post_typed(
            &format!("/v1/threads/{thread_id}/rerun"),
            &body,
            Some(revision),
            &key,
        )?;
        Ok((
            ScopeId::from(value["scope_id"].as_str().unwrap_or_default().to_string()),
            value["generation"].as_u64().unwrap_or_default(),
        ))
    }

    /// 对比同一 Thread 的两个 Scope。
    pub fn compare(&self, thread_id: &ThreadId) -> Result<String> {
        let value: Value = self.get_typed(&format!("/v1/threads/{thread_id}/compare"))?;
        Ok(value["comparison"].as_str().unwrap_or_default().to_string())
    }

    /// 资源账：预算、累计用量与 Sandbox 证据。
    pub fn resources(&self, thread_id: &ThreadId) -> Result<ResourceView> {
        self.get_typed(&format!("/v1/threads/{thread_id}/resources"))
    }

    /// 配置 Opaque Harness Profile。
    pub fn configure_opaque_harness(
        &self,
        thread_id: &ThreadId,
        policy: OpaqueHarnessPolicy,
    ) -> Result<ThreadSpec> {
        let revision = self.revision_for(thread_id)?;
        let body = json!({
            "enabled": policy.enabled,
            "allow_native_provider_network": policy.allow_native_provider_network,
            "use_tool_gateway": policy.use_tool_gateway,
            "max_observed_effect": side_effect_name(policy.max_observed_effect),
        });
        let key = operation_key(&["opaque", &thread_id.to_string(), &revision.to_string()]);
        let spec: ThreadSpec = self.post_typed(
            &format!("/v1/threads/{thread_id}/opaque"),
            &body,
            Some(revision),
            &key,
        )?;
        self.remember_revision(thread_id, spec.revision);
        Ok(spec)
    }

    // ——————————————————————————— 世界与身份 ———————————————————————————

    /// 个人简报。
    pub fn brief(&self) -> Result<PersonalBrief> {
        self.get_typed("/v1/world/brief")
    }

    /// 一个 Thread 的 World 条目。
    pub fn world_items(
        &self,
        thread_id: &ThreadId,
        include_resolved: bool,
    ) -> Result<Vec<WorldItem>> {
        let path = if include_resolved {
            format!("/v1/threads/{thread_id}/world?include_resolved=1")
        } else {
            format!("/v1/threads/{thread_id}/world")
        };
        self.get_typed(&path)
    }

    /// 解决一个 World 条目。
    pub fn resolve_world_item(
        &self,
        thread_id: &ThreadId,
        item_id: &WorldItemId,
        kind: WorldItemResolutionKind,
        note: Option<&str>,
    ) -> Result<WorldItem> {
        let revision = self.revision_for(thread_id)?;
        let body = json!({
            "item_id": item_id.to_string(),
            "kind": kind,
            "note": note,
        });
        let key = operation_key(&[
            "world-resolve",
            &thread_id.to_string(),
            &item_id.to_string(),
            &format!("{kind:?}"),
        ]);
        self.post_typed(
            &format!("/v1/threads/{thread_id}/world/resolve"),
            &body,
            Some(revision),
            &key,
        )
    }

    /// Project 世界。
    pub fn project_worlds(
        &self,
        thread_id: Option<&ThreadId>,
        include_resolved: bool,
    ) -> Result<Vec<ProjectWorld>> {
        let mut path = "/v1/projects".to_string();
        if let Some(thread_id) = thread_id {
            path.push_str(&format!("?thread_id={thread_id}"));
        }
        if include_resolved {
            path.push(if path.contains('?') { '&' } else { '?' });
            path.push_str("include_resolved=1");
        }
        self.get_typed(&path)
    }

    /// 身份世界：联系人、客户、线程与里程碑。
    pub fn identity_worlds(
        &self,
        thread_id: Option<&ThreadId>,
        include_resolved: bool,
    ) -> Result<Vec<IdentityWorld>> {
        let mut path = "/v1/identities".to_string();
        if let Some(thread_id) = thread_id {
            path.push_str(&format!("?thread_id={thread_id}"));
        }
        if include_resolved {
            path.push(if path.contains('?') { '&' } else { '?' });
            path.push_str("include_resolved=1");
        }
        self.get_typed(&path)
    }

    /// 纠正身份关联：Merge / Split / Rename。
    pub fn correct_world_correlation(
        &self,
        thread_id: &ThreadId,
        correction: WorldCorrelationCorrection,
    ) -> Result<WorldCorrelationCorrection> {
        let revision = self.revision_for(thread_id)?;
        let body = serde_json::to_value(&correction)
            .map_err(|error| ClientError::Malformed(error.to_string()))?;
        let key = operation_key(&[
            "world-correction",
            &thread_id.to_string(),
            &revision.to_string(),
        ]);
        self.post_typed(
            &format!("/v1/threads/{thread_id}/world/corrections"),
            &body,
            Some(revision),
            &key,
        )
    }

    // ——————————————————————————— 信号与候选 ———————————————————————————

    /// 读取 World Signal。
    pub fn world_signals(
        &self,
        cursor: u64,
        provider: Option<&str>,
        entity: Option<&str>,
    ) -> Result<Vec<WorldSignalRecord>> {
        let mut path = format!("/v1/signals?cursor={cursor}");
        if let Some(provider) = provider {
            path.push_str(&format!("&provider={provider}"));
        }
        if let Some(entity) = entity {
            path.push_str(&format!("&entity={entity}"));
        }
        self.get_typed(&path)
    }

    /// 解析 Signal，产出 Thread 候选。
    pub fn resolve_signals(&self, thread_id: &ThreadId) -> Result<Vec<ThreadCandidate>> {
        let revision = self.revision_for(thread_id)?;
        let key = operation_key(&[
            "signals-resolve",
            &thread_id.to_string(),
            &revision.to_string(),
        ]);
        self.post_typed(
            &format!("/v1/threads/{thread_id}/signals/resolve"),
            &json!({}),
            Some(revision),
            &key,
        )
    }

    /// 读取 Thread 候选。
    pub fn thread_candidates(&self, cursor: u64) -> Result<Vec<ThreadCandidateRecord>> {
        self.get_typed(&format!("/v1/candidates?cursor={cursor}"))
    }

    /// 读取一个 Thread 名下的候选。
    pub fn thread_candidates_of(&self, thread_id: &ThreadId) -> Result<Vec<ThreadCandidateRecord>> {
        self.get_typed(&format!("/v1/threads/{thread_id}/candidates"))
    }

    /// 读取一个 Thread 收到的 World Signal。
    pub fn world_signals_of(&self, thread_id: &ThreadId) -> Result<Vec<WorldSignal>> {
        self.get_typed(&format!("/v1/threads/{thread_id}/signals"))
    }

    /// 提出一个 Thread 候选。
    pub fn propose_thread_candidate(
        &self,
        thread_id: &ThreadId,
        candidate: ThreadCandidate,
    ) -> Result<ThreadCandidate> {
        let revision = self.revision_for(thread_id)?;
        let body = serde_json::to_value(&candidate)
            .map_err(|error| ClientError::Malformed(error.to_string()))?;
        let key = operation_key(&["candidate", &thread_id.to_string(), &revision.to_string()]);
        self.post_typed(
            &format!("/v1/threads/{thread_id}/candidates"),
            &body,
            Some(revision),
            &key,
        )
    }

    /// 接受或拒绝一个候选。
    pub fn resolve_thread_candidate(
        &self,
        thread_id: &ThreadId,
        candidate_id: &ThreadCandidateId,
        accept: bool,
    ) -> Result<(ThreadCandidateRecord, Option<ThreadSpec>)> {
        let revision = self.revision_for(thread_id)?;
        let action = if accept { "accept" } else { "reject" };
        let key = operation_key(&[
            &format!("candidate-{action}"),
            &thread_id.to_string(),
            &candidate_id.to_string(),
            &revision.to_string(),
        ]);
        let value: Value = self.post_typed(
            &format!("/v1/threads/{thread_id}/candidates/{candidate_id}/{action}"),
            &json!({}),
            Some(revision),
            &key,
        )?;
        let candidate: ThreadCandidateRecord =
            serde_json::from_value(value["candidate"].clone())
                .map_err(|error| ClientError::Malformed(error.to_string()))?;
        let thread = match &value["thread"] {
            Value::Null => None,
            other => Some(
                serde_json::from_value(other.clone())
                    .map_err(|error| ClientError::Malformed(error.to_string()))?,
            ),
        };
        Ok((candidate, thread))
    }

    // ——————————————————————————— Connector ———————————————————————————

    /// 可用的 Connector Adapter 清单。
    pub fn connector_manifests(&self) -> Result<Vec<ConnectorManifest>> {
        self.get_typed("/v1/connector-adapters")
    }

    /// 注册一个 Connector。
    pub fn register_connector(
        &self,
        thread_id: &ThreadId,
        name: &str,
        source: &str,
        adapter: &str,
        credential: Option<&ConnectorCredential>,
        sync_interval_seconds: u64,
    ) -> Result<ConnectorRegistration> {
        let revision = self.revision_for(thread_id)?;
        let body = json!({
            "name": name,
            "source": source,
            "adapter": adapter,
            "credential": credential,
            "sync_interval_seconds": sync_interval_seconds,
        });
        let key = operation_key(&[
            "connector",
            &thread_id.to_string(),
            &revision.to_string(),
            name,
            source,
        ]);
        self.post_typed(
            &format!("/v1/threads/{thread_id}/connectors"),
            &body,
            Some(revision),
            &key,
        )
    }

    /// 列出 Thread 的 Connector。
    pub fn connectors(&self, thread_id: &ThreadId) -> Result<Vec<ConnectorRegistration>> {
        self.get_typed(&format!("/v1/threads/{thread_id}/connectors"))
    }

    /// Connector 健康状况。
    pub fn connector_health(
        &self,
        thread_id: &ThreadId,
        connector_id: &ConnectorId,
    ) -> Result<ConnectorHealthReport> {
        self.get_typed(&format!(
            "/v1/threads/{thread_id}/connectors/{connector_id}/health"
        ))
    }

    /// 替换 Connector 凭据。
    pub fn attach_connector_credential(
        &self,
        thread_id: &ThreadId,
        connector_id: &ConnectorId,
        credential: ConnectorCredential,
    ) -> Result<ConnectorCredentialMetadata> {
        let revision = self.revision_for(thread_id)?;
        let body = serde_json::to_value(&credential)
            .map_err(|error| ClientError::Malformed(error.to_string()))?;
        let key = operation_key(&[
            "credential",
            &thread_id.to_string(),
            &connector_id.to_string(),
            &revision.to_string(),
        ]);
        self.post_typed(
            &format!("/v1/threads/{thread_id}/connectors/{connector_id}/credential"),
            &body,
            Some(revision),
            &key,
        )
    }

    /// 撤销 Connector 凭据。
    pub fn revoke_connector_credential(
        &self,
        thread_id: &ThreadId,
        connector_id: &ConnectorId,
    ) -> Result<ConnectorCredentialMetadata> {
        let revision = self.revision_for(thread_id)?;
        let key = operation_key(&[
            "credential-revoke",
            &thread_id.to_string(),
            &connector_id.to_string(),
            &revision.to_string(),
        ]);
        self.post_typed(
            &format!("/v1/threads/{thread_id}/connectors/{connector_id}/credential"),
            &json!({ "revoke": true }),
            Some(revision),
            &key,
        )
    }

    /// 立即同步一个 Connector。
    pub fn sync_connector(
        &self,
        thread_id: &ThreadId,
        connector_id: &ConnectorId,
    ) -> Result<ConnectorSyncResult> {
        let revision = self.revision_for(thread_id)?;
        let key = operation_key(&[
            "sync",
            &thread_id.to_string(),
            &connector_id.to_string(),
            &revision.to_string(),
        ]);
        self.post_typed(
            &format!("/v1/threads/{thread_id}/connectors/{connector_id}/sync"),
            &json!({}),
            Some(revision),
            &key,
        )
    }

    /// 重建游标并重新同步。
    pub fn repair_connector(
        &self,
        thread_id: &ThreadId,
        connector_id: &ConnectorId,
    ) -> Result<ConnectorRepairReport> {
        let revision = self.revision_for(thread_id)?;
        let key = operation_key(&[
            "repair",
            &thread_id.to_string(),
            &connector_id.to_string(),
            &revision.to_string(),
        ]);
        self.post_typed(
            &format!("/v1/threads/{thread_id}/connectors/{connector_id}/repair"),
            &json!({}),
            Some(revision),
            &key,
        )
    }

    /// 配置 Connector 的受限自动接受策略。
    pub fn configure_connector_policy(
        &self,
        thread_id: &ThreadId,
        policy: else_protocol::connector::ConnectorPolicy,
    ) -> Result<ThreadSpec> {
        let revision = self.revision_for(thread_id)?;
        let body = serde_json::to_value(&policy)
            .map_err(|error| ClientError::Malformed(error.to_string()))?;
        let key = operation_key(&[
            "connector-policy",
            &thread_id.to_string(),
            &revision.to_string(),
        ]);
        let spec: ThreadSpec = self.post_typed(
            &format!("/v1/threads/{thread_id}/connector-policy"),
            &body,
            Some(revision),
            &key,
        )?;
        self.remember_revision(thread_id, spec.revision);
        Ok(spec)
    }

    // ——————————————————————————— 外部动作 ———————————————————————————

    /// 列出 Connector 动作。
    pub fn connector_actions(&self, thread_id: &ThreadId) -> Result<Vec<ConnectorAction>> {
        self.get_typed(&format!("/v1/threads/{thread_id}/actions"))
    }

    /// 提出一个外部动作。
    pub fn propose_connector_action(
        &self,
        thread_id: &ThreadId,
        connector_id: &ConnectorId,
        capability: &str,
        target: &str,
        input: Value,
        idempotency_key: Option<&str>,
    ) -> Result<ConnectorAction> {
        let revision = self.revision_for(thread_id)?;
        let body = json!({
            "connector_id": connector_id.to_string(),
            "capability": capability,
            "target": target,
            "input": input,
            "idempotency_key": idempotency_key,
        });
        let key = operation_key(&[
            "action",
            &thread_id.to_string(),
            &connector_id.to_string(),
            capability,
            target,
        ]);
        self.post_typed(
            &format!("/v1/threads/{thread_id}/actions"),
            &body,
            Some(revision),
            &key,
        )
    }

    /// 执行 / 核验 / 补偿一个外部动作。
    pub fn connector_action(
        &self,
        thread_id: &ThreadId,
        action_id: &ConnectorActionId,
        stage: &str,
    ) -> Result<ConnectorAction> {
        let revision = self.revision_for(thread_id)?;
        let key = operation_key(&[
            &format!("action-{stage}"),
            &thread_id.to_string(),
            &action_id.to_string(),
            &revision.to_string(),
        ]);
        self.post_typed(
            &format!("/v1/threads/{thread_id}/actions/{action_id}/{stage}"),
            &json!({}),
            Some(revision),
            &key,
        )
    }

    // ——————————————————————————— 能力与 Harness ———————————————————————————

    /// 可用 Harness 清单。
    pub fn harnesses(&self) -> Result<Vec<HarnessView>> {
        self.get_typed("/v1/harnesses")
    }

    /// Thread 的能力注册表。
    pub fn capabilities(&self, thread_id: &ThreadId) -> Result<Vec<CapabilityRegistration>> {
        self.get_typed(&format!("/v1/threads/{thread_id}/capabilities"))
    }

    /// Thread 的 Agent 绑定。
    pub fn agent_bindings(&self, thread_id: &ThreadId) -> Result<Vec<AgentBinding>> {
        self.get_typed(&format!("/v1/threads/{thread_id}/bindings"))
    }

    // ——————————————————————————— 事件 ———————————————————————————

    /// 一个 Thread 的事件。
    pub fn events(&self, thread_id: &ThreadId) -> Result<Vec<EventResponse>> {
        self.get_typed(&format!("/v1/threads/{thread_id}/events"))
    }

    /// 全局事件增量。
    pub fn global_events(&self, cursor: u64) -> Result<Vec<EventResponse>> {
        self.get_typed(&format!("/v1/events?cursor={cursor}"))
    }

    // ——————————————————————————— 系统 ———————————————————————————

    /// 导出全部数据。
    pub fn export_data(&self) -> Result<Value> {
        self.get_value("/v1/export")
    }

    /// 诊断报告。
    pub fn diagnostics(&self) -> Result<Value> {
        self.get_value("/v1/diagnostics")
    }

    /// Runtime 设置。
    pub fn settings(&self) -> Result<RuntimeSettings> {
        self.get_typed("/v1/settings")
    }

    /// 更新 Runtime 设置。
    pub fn update_settings(&self, settings: &RuntimeSettings) -> Result<RuntimeSettings> {
        let body = serde_json::to_value(settings)
            .map_err(|error| ClientError::Malformed(error.to_string()))?;
        let key = unique_key("settings");
        self.post_typed("/v1/settings", &body, None, &key)
    }

    /// 清理孤儿 Lease。
    pub fn cleanup_orphan_leases(&self) -> Result<Value> {
        let key = unique_key("cleanup-leases");
        self.post_value("/v1/diagnostics/leases/cleanup", &json!({}), None, &key)
    }

    /// 重试失败的 Scope。
    pub fn retry_failed_scopes(&self, thread_id: &ThreadId) -> Result<Value> {
        let revision = self.revision_for(thread_id)?;
        let key = operation_key(&["retry", &thread_id.to_string(), &revision.to_string()]);
        self.post_value(
            &format!("/v1/threads/{thread_id}/repair/retry"),
            &json!({}),
            Some(revision),
            &key,
        )
    }

    /// 查询一个异步操作的状态。
    pub fn operation(&self, operation_id: &str) -> Result<RuntimeOperation> {
        self.get_typed(&format!("/v1/operations/{operation_id}"))
    }

    /// Runtime 是否健康。
    pub fn health(&self) -> Result<bool> {
        let value = self.get_value("/v1/health")?;
        Ok(value["ok"].as_bool().unwrap_or(false))
    }

    /// 契约版本。
    ///
    /// 这是**鉴权**端点：能拿到它，就说明连的是正确的 Runtime 且令牌有效 ——
    /// 公开的 `/v1/health` 做不到这一点。
    pub fn version(&self) -> Result<RuntimeVersion> {
        self.get_typed("/v1/version")
    }

    /// 打开事件流。
    ///
    /// 令牌留在客户端内部：surface 拿到的是事件流，不是凭据。
    pub fn event_stream(&self, cursor: u64) -> Result<crate::stream::EventStream> {
        crate::stream::EventStream::connect(&self.base, &self.token, cursor)
    }

    /// 确认「连的是正确的 Runtime，且令牌有效」。
    ///
    /// 公开的 `/v1/health` 对任何人都返回 200，用它判断连接会把**另一个**
    /// Runtime（令牌不同）误认成自己的。这里用鉴权端点：优先读契约版本，
    /// 旧版 Runtime 没有该路由时退回鉴权列表接口。
    pub fn probe(&self) -> Result<()> {
        if self.version().is_ok() {
            return Ok(());
        }
        self.list_threads().map(|_| ())
    }
}

/// 幂等键：同一个逻辑写操作在任何重试里都用同一个键。
///
/// daemon 见到重复的键会返回缓存的响应，不会重复执行副作用。
/// 把 Thread 的 Revision 放进键里，状态一变键就变 —— 于是「重试」安全，
/// 而「新的一次操作」不会被旧结果吞掉。
fn operation_key(parts: &[&str]) -> String {
    parts
        .iter()
        .map(|part| header_safe(part))
        .collect::<Vec<_>>()
        .join(":")
}

/// 把一段文本变成 header 里安全的片段。
///
/// 键最终要放进 HTTP header，值只能是 ASCII 可见字符。自由文本（任务描述、
/// 事件摘要）常常是中文，所以换成稳定的短哈希：既 header 安全，
/// 又保持「同样的输入 → 同样的键」。
fn header_safe(part: &str) -> String {
    if !part.is_empty()
        && part
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return part.to_string();
    }
    format!("h{:016x}", fnv1a(part.as_bytes()))
}

/// FNV-1a：短、稳定、够用的非密码学哈希。
fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// 一次性操作的幂等键。
///
/// 这类操作每次调用都应当真的重做（创建 Thread、改设置、清 Lease），
/// 所以键里带一个随机量，避免被上一次的结果命中。
fn unique_key(prefix: &str) -> String {
    format!("{prefix}:{}", uuid::Uuid::new_v4())
}

/// 副作用等级的线上写法。
fn side_effect_name(kind: SideEffectKind) -> &'static str {
    match kind {
        SideEffectKind::ReadOnly => "read_only",
        SideEffectKind::Reversible => "reversible",
        SideEffectKind::Compensatable => "compensatable",
        SideEffectKind::Irreversible => "irreversible",
        SideEffectKind::Unknown => "unknown",
    }
}

/// 记忆作用域的线上写法。
fn memory_scope_name(scope: &MemoryScope) -> String {
    match scope {
        MemoryScope::Global => "global".to_string(),
        MemoryScope::Project(project) => format!("project:{project}"),
        MemoryScope::Thread(thread_id) => format!("thread:{thread_id}"),
    }
}
