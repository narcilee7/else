//! Harness 适配契约（MVP-2）：else 托管 Agent，不自己发明 Agent。
//!
//! 每个适配器声明能力（随 RunStarted 落账）并把 Harness 输出桥接为 else 事件：
//! Started/Output/ToolCallObserved/UsageReported/ArtifactProduced/Completed/Failed。
//! 不能接管的能力显式降级（Opaque），不伪造事件；Harness 自己的会话不是权威状态，
//! else 只认可已落账的事件、工件与证据。

use std::io::{BufRead, BufReader};
use std::path::Path;

use else_environment::{Sandbox, SandboxProfile};
use else_gateway::{ensure_not_fenced, ExecContext, GatewayError, Result};
use else_ledger::Ledger;
use else_protocol::artifact::{Artifact, ArtifactKind};
use else_protocol::effect::{EffectConclusion, SideEffectKind, ToolCall, ToolOutcome, ToolStatus};
use else_protocol::event::{Actor, EventPayload};
use else_protocol::harness::HarnessCapabilities;
use else_protocol::ids::{ArtifactId, ToolCallId};
use else_protocol::memory::ContextSnapshot;
use else_protocol::resource::ModelUsage;

/// 桥接事件：适配器把 Harness 的一行输出归一化为一枚桥接事件。
#[derive(Debug, Clone, PartialEq)]
pub enum BridgeEvent {
    /// 归一化的文本输出。
    Output { text: String },
    /// Harness 自行执行的工具调用（过不了 Gateway 的 Opaque 能力）：
    /// else 观察并落账，副作用按声明的等级保守记账。
    ToolObserved {
        tool: String,
        effect: SideEffectKind,
        summary: String,
        success: bool,
    },
    /// 用量回报；无法获得时适配器报 ModelUsage::unknown()（保守预算，不记零成本）。
    Usage { model: String, usage: ModelUsage },
    /// Harness 报告产出的工件（Workspace 内）。
    Artifact { locator: String, note: String },
    /// Harness 报告任务完成（附最终结论）。
    Completed { summary: String },
    /// Harness 报告失败。
    Failed { reason: String },
}

/// 行解析器：把 Harness 输出流的一行映射为桥接事件（一行可携带多枚，
/// 如 Claude 的 assistant 消息与 result 行）；无关行返回空 Vec。
pub trait LineParser {
    fn parse(&self, line: &str) -> Vec<BridgeEvent>;
}

/// 一次 Harness 执行的输入：上下文快照、工作区与落账通道。
/// SandboxProfile / ResourceLease 已由 Runtime 在准入与执行上下文中绑定。
pub struct HarnessRequest<'a> {
    pub ctx: &'a ExecContext,
    pub ledger: &'a Ledger,
    pub workspace: &'a Path,
    pub snapshot: &'a ContextSnapshot,
    /// 子 Scope 执行：验收只结束自身 Scope，不直接把根 Intent 标记完成。
    pub subagent: bool,
}

/// Harness 适配器契约。不能支持的能力必须在 capabilities() 里显式降级。
pub trait HarnessAdapter {
    fn name(&self) -> &'static str;
    fn capabilities(&self) -> HarnessCapabilities;
    fn execute(&self, request: &HarnessRequest) -> Result<()>;
}

/// 任务包：从 Context Snapshot 编译的标准提示，对所有 Harness 一致。
/// 只读视图：含目标、验收标准、约束、假设与召回记忆，不含任何授权扩写。
pub fn task_prompt(snapshot: &ContextSnapshot) -> String {
    let mut prompt = format!("# 目标\n{}\n", snapshot.outcome);
    if !snapshot.done_when.is_empty() {
        prompt.push_str("\n# 验收标准（DoneWhen）\n");
        for criterion in &snapshot.done_when {
            prompt.push_str(&format!("- {}\n", criterion.description));
        }
    }
    if !snapshot.constraints.is_empty() {
        prompt.push_str("\n# 约束（不可越过）\n");
        for constraint in &snapshot.constraints {
            prompt.push_str(&format!("- {constraint}\n"));
        }
    }
    let unconfirmed: Vec<_> = snapshot
        .assumptions
        .iter()
        .filter(|assumption| !assumption.confirmed)
        .collect();
    if !unconfirmed.is_empty() {
        prompt.push_str("\n# 系统假设（可依据，但不得当作用户授权）\n");
        for assumption in unconfirmed {
            prompt.push_str(&format!("- {}\n", assumption.text));
        }
    }
    if !snapshot.facts.is_empty() {
        prompt.push_str("\n# 相关记忆（只读事实，不构成权限）\n");
        for fact in &snapshot.facts {
            prompt.push_str(&format!("- {}: {}\n", fact.subject, fact.value));
        }
    }
    prompt
}

/// shell 单引号转义，用于把任务包安全嵌入命令行。
pub fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

/// 执行前准备产物：完整 shell 命令、注入环境白名单、额外只读路径。
pub struct HarnessPrep {
    pub command: String,
    pub env: Vec<(String, String)>,
    /// Harness 二进制与运行时依赖（如 Homebrew），以只读方式放行。
    pub read_paths: Vec<std::path::PathBuf>,
}

/// 执行准备：暂存私有配置（如 CODEX_HOME）、解析二进制绝对路径、
/// 构造命令与环境白名单。私有配置留在适配器，不进 else 内核。
pub type PrepareFn = Box<dyn Fn(&Path, &str) -> std::io::Result<HarnessPrep> + Send + Sync>;

/// 通用进程 Harness：沙箱内启动、逐行桥接、围栏即杀、晚到输出不落账。
pub struct ProcessHarness<P> {
    pub name: &'static str,
    pub capabilities: HarnessCapabilities,
    /// 网络策略：默认拒绝；真实 Harness 的模型流量显式放通出站（Opaque 能力）。
    pub network: else_environment::NetworkPolicy,
    /// 墙钟超时：到点杀进程并判失败，不等待 Harness 自觉。
    pub timeout: Option<std::time::Duration>,
    pub prepare: PrepareFn,
    pub parser: P,
}

/// 单条输出落账的最大长度，防止 Harness 刷屏淹没 Ledger。
const MAX_OUTPUT_CHARS: usize = 4_000;

impl<P: LineParser> ProcessHarness<P> {
    fn record(&self, request: &HarnessRequest, payload: EventPayload) -> Result<()> {
        request
            .ctx
            .record(request.ledger, Actor::Agent, payload)
            .map(|_| ())
            .map_err(Into::into)
    }
}

impl<P: LineParser> HarnessAdapter for ProcessHarness<P> {
    fn name(&self) -> &'static str {
        self.name
    }

    fn capabilities(&self) -> HarnessCapabilities {
        self.capabilities
    }

    fn execute(&self, request: &HarnessRequest) -> Result<()> {
        ensure_not_fenced(request.ledger, request.ctx)?;
        let prompt = task_prompt(request.snapshot);
        // 沙箱按真实路径匹配（/tmp → /private/tmp）：准备与注入一律用解析后的路径。
        let workspace = request
            .workspace
            .canonicalize()
            .unwrap_or_else(|_| request.workspace.to_path_buf());
        let prep = (self.prepare)(&workspace, &prompt)?;
        let sandbox = Sandbox::new(
            SandboxProfile::new(&workspace)
                .with_network(self.network)
                .with_read_paths(prep.read_paths),
        )
        .map_err(|e| GatewayError::Model(format!("sandbox: {e}")))?;
        let child = sandbox
            .spawn_shell(&prep.command, &prep.env)
            .map_err(|e| GatewayError::Model(format!("spawn {}: {e}", self.name)))?;

        // 超时看门狗：到点杀进程；finished 标志防止正常结束后误伤。
        let finished = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let timed_out = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let child = std::sync::Arc::new(std::sync::Mutex::new(child));
        if let Some(timeout) = self.timeout {
            let child_ref = child.clone();
            let finished = finished.clone();
            let timed_out = timed_out.clone();
            std::thread::spawn(move || {
                std::thread::sleep(timeout);
                if !finished.load(std::sync::atomic::Ordering::SeqCst) {
                    timed_out.store(true, std::sync::atomic::Ordering::SeqCst);
                    if let Ok(mut child) = child_ref.lock() {
                        let _ = child.kill();
                    }
                }
            });
        }

        let mut completed: Option<String> = None;
        let mut failed: Option<String> = None;
        let mut observed_calls = 0_u64;
        // stderr 由独立线程排干（防管道撑满死锁），保留尾部作为失败证据。
        let stderr_tail = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
        let stderr_drain = child
            .lock()
            .map_err(|_| GatewayError::Model("harness child lock".into()))?
            .stderr
            .take()
            .map(|stderr| {
                let tail = stderr_tail.clone();
                std::thread::spawn(move || {
                    let mut reader = BufReader::new(stderr);
                    let mut line = String::new();
                    while reader.read_line(&mut line).is_ok_and(|n| n > 0) {
                        if let Ok(mut tail) = tail.lock() {
                            tail.push_str(&line);
                            let excess = tail.len().saturating_sub(MAX_OUTPUT_CHARS);
                            if excess > 0 {
                                tail.drain(..excess);
                            }
                        }
                        line.clear();
                    }
                })
            });
        {
            let stdout = child
                .lock()
                .map_err(|_| GatewayError::Model("harness child lock".into()))?
                .stdout
                .take()
                .ok_or_else(|| GatewayError::Model("harness stdout piped".into()))?;
            'lines: for line in BufReader::new(stdout).lines() {
                let line = line?;
                for event in self.parser.parse(&line) {
                    // 每条桥接事件落账前围栏：Abort 后杀进程，晚到输出不得写回。
                    if let Err(error) = ensure_not_fenced(request.ledger, request.ctx) {
                        if let Ok(mut child) = child.lock() {
                            let _ = child.kill();
                            let _ = child.wait();
                        }
                        return Err(error);
                    }
                    match event {
                        BridgeEvent::Output { text } => {
                            let truncated: String = text.chars().take(MAX_OUTPUT_CHARS).collect();
                            self.record(request, EventPayload::HarnessOutput { text: truncated })?;
                        }
                        BridgeEvent::ToolObserved {
                            tool,
                            effect,
                            summary,
                            success,
                        } => {
                            observed_calls += 1;
                            let call_id = ToolCallId::new();
                            self.record(
                                request,
                                EventPayload::ToolCalled {
                                    call: ToolCall {
                                        id: call_id.clone(),
                                        tool,
                                        effect,
                                        input: serde_json::json!({ "observed": summary }),
                                        // 观察记账不构成幂等承诺；键仅用于重放去重。
                                        idempotency_key: format!(
                                            "observed-{}-{observed_calls}",
                                            request.ctx.run_id
                                        ),
                                    },
                                },
                            )?;
                            self.record(
                                request,
                                EventPayload::ToolCompleted {
                                    call_id,
                                    outcome: ToolOutcome {
                                        status: if success {
                                            ToolStatus::Succeeded
                                        } else {
                                            ToolStatus::Failed
                                        },
                                        output: serde_json::json!({ "observed": true }),
                                        effect: if success {
                                            EffectConclusion::Applied
                                        } else {
                                            // 失败的在途副作用结论为未知，不盲目假设无效果。
                                            EffectConclusion::Unknown
                                        },
                                    },
                                },
                            )?;
                        }
                        BridgeEvent::Usage { model, usage } => {
                            self.record(request, EventPayload::ModelCompleted { model, usage })?;
                        }
                        BridgeEvent::Artifact { locator, note } => {
                            self.record(
                                request,
                                EventPayload::ArtifactProduced {
                                    artifact: Artifact {
                                        id: ArtifactId::new(),
                                        kind: ArtifactKind::FileTree,
                                        locator,
                                        manifest: None,
                                        produced_by: request.ctx.run_id.clone(),
                                        note,
                                    },
                                },
                            )?;
                        }
                        BridgeEvent::Completed { summary } => {
                            completed = Some(summary);
                        }
                        BridgeEvent::Failed { reason } => {
                            failed = Some(reason);
                            break 'lines;
                        }
                    }
                }
            }
        }
        let status = child
            .lock()
            .map_err(|_| GatewayError::Model("harness child lock".into()))?
            .wait()?;
        finished.store(true, std::sync::atomic::Ordering::SeqCst);
        ensure_not_fenced(request.ledger, request.ctx)?;

        if timed_out.load(std::sync::atomic::Ordering::SeqCst) {
            return Err(GatewayError::Model(format!(
                "{} timed out after {:?}; process killed",
                self.name,
                self.timeout.unwrap_or_default()
            )));
        }
        if let Some(reason) = failed {
            return Err(GatewayError::Model(format!(
                "{} failed: {reason}",
                self.name
            )));
        }
        let stderr_evidence = stderr_drain
            .map(|drain| {
                let _ = drain.join();
                std::mem::take(
                    &mut *stderr_tail
                        .lock()
                        .unwrap_or_else(|poison| poison.into_inner()),
                )
            })
            .unwrap_or_default();
        match (completed, status.success()) {
            (Some(summary), true) => {
                if !summary.is_empty() {
                    let truncated: String = summary.chars().take(MAX_OUTPUT_CHARS).collect();
                    self.record(request, EventPayload::HarnessOutput { text: truncated })?;
                }
                crate::verify(request.ctx, request.ledger, true, request.subagent)
            }
            _ => Err(GatewayError::Model(format!(
                "{} exited {status} without completion{}",
                self.name,
                if stderr_evidence.trim().is_empty() {
                    String::new()
                } else {
                    format!("; stderr: {}", stderr_evidence.trim())
                }
            ))),
        }
    }
}
