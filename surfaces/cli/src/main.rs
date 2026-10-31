//! else CLI：Runtime API 的第一个薄客户端。
//! 只做参数解析与打印，不含业务逻辑。

use std::collections::BTreeSet;
use std::path::PathBuf;

use clap::{Parser, Subcommand};
use else_api::Runtime;
use else_protocol::attention::DecisionChoice;
use else_protocol::ids::{AttentionId, IntentId};

#[derive(Parser)]
#[command(name = "else", about = "You do what matters. Else handles the rest.")]
struct Cli {
    /// Runtime 数据目录（事件库 + 工作区）
    #[arg(long, default_value = ".else")]
    dir: PathBuf,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// 提交一个抽象 Intent 并启动执行
    Intent {
        outcome: String,
        /// 项目作用域：历史结论与纠正按项目召回，不泛化到全局
        #[arg(long)]
        project: Option<String>,
        /// 执行器：auto（默认）/ scripted / codex / claude-code
        #[arg(long, default_value = "auto")]
        harness: String,
        /// 显式允许不可接管 Harness 执行观察式副作用。
        #[arg(long)]
        allow_opaque: bool,
        /// 显式允许 Opaque Harness 使用原生 Provider 出站网络。
        #[arg(long)]
        opaque_network: bool,
        /// 通过 loopback Tool Gateway 执行文件与 Shell，父 Harness Workspace 只读。
        #[arg(long)]
        tool_gateway: bool,
        /// Opaque Profile 允许的最高观察副作用等级。
        #[arg(long, default_value = "read_only", value_parser = ["read_only", "reversible", "compensatable", "irreversible", "unknown"])]
        opaque_max_effect: String,
    },
    /// 让 Kernel 从持久状态推导并执行下一步
    Advance {
        intent_id: String,
        /// 执行器偏好；auto 仍由 Kernel 按规则选择
        #[arg(long, default_value = "auto")]
        harness: String,
    },
    /// 连续推进 Kernel，直到等待用户、Run、Timer 或空闲
    Drive {
        intent_id: String,
        #[arg(long, default_value = "auto")]
        harness: String,
        #[arg(long, default_value_t = 8)]
        max_steps: u64,
    },
    /// 记录一个外部事件并唤醒该 Intent
    Event {
        intent_id: String,
        source: String,
        summary: String,
    },
    /// 标记 Intent 正在等待外部事实，停止空转执行
    WaitExternal {
        intent_id: String,
        #[arg(long, default_value = "waiting for external event")]
        reason: String,
    },
    /// 安排一个持久化 Kernel 唤醒 Timer
    Wake {
        intent_id: String,
        /// 从现在起多少秒触发。
        seconds: i64,
        #[arg(long, default_value = "follow up")]
        reason: String,
    },
    /// 对已有 Intent 再启动一次执行
    Run {
        intent_id: String,
        /// 执行器：scripted 或已注册的真实 Harness
        #[arg(long, default_value = "scripted")]
        harness: String,
    },
    /// 列出可用的 Harness 适配器及其能力声明
    Harnesses,
    /// 为已有 Intent 显式启用 Opaque Harness Profile
    AllowOpaque {
        intent_id: String,
        #[arg(long)]
        network: bool,
        #[arg(long)]
        tool_gateway: bool,
        #[arg(long, default_value = "read_only", value_parser = ["read_only", "reversible", "compensatable", "irreversible", "unknown"])]
        max_effect: String,
    },
    /// 取消执行并收敛在途副作用（可安全重复调用）
    Abort {
        intent_id: String,
        reason: Option<String>,
    },
    /// 列出全部 Intent
    List,
    /// 查看 Scope 委托树与生命周期
    Scopes { intent_id: String },
    /// 委托一个子 Scope（权限只能低于或等于父级）
    Delegate {
        intent_id: String,
        parent_scope: String,
        task: String,
        /// 显式执行依赖；所有依赖 Scope Completed 后才会调度
        #[arg(long = "depends-on")]
        depends_on: Vec<String>,
        #[arg(long, default_value = "read_only", value_parser = ["read_only", "reversible", "compensatable", "irreversible", "unknown"])]
        max_effect: String,
    },
    /// 启动或续跑指定 Scope；--harness auto 按规则选择执行器（MVP-3 Dispatch）
    RunScope {
        intent_id: String,
        scope_id: String,
        /// 执行器：scripted（默认）/ auto / codex / claude-code
        #[arg(long, default_value = "scripted")]
        harness: String,
    },
    /// 查看等待本人的 Decision
    Decisions { intent_id: String },
    /// 查看 Attention Window、批量率与误打断指标
    Attention { intent_id: String },
    /// 查看结构化 Plan 版本、步骤与外部事实影响
    Plan { intent_id: String },
    /// 回答一个 Decision（选项 id 或自由文本）
    Decide {
        intent_id: String,
        request_id: String,
        choice: String,
    },
    /// 查看事件流（调试视图）
    Log { intent_id: String },
    /// 查看模型、工具与环境资源预算及实际消耗
    Resources { intent_id: String },
    /// 创建检查点（事件水位 + 工作区快照 + 在途副作用结论）
    Checkpoint { intent_id: String },
    /// 从检查点派生分支并重跑（--alternative 走替代路径）
    Rerun {
        intent_id: String,
        checkpoint_id: String,
        #[arg(long)]
        alternative: bool,
    },
    /// 按 Generation 比较各执行分支（Outcome/副作用/产物）
    Compare { intent_id: String },
    /// 修订 Intent 目标（revision +1）；运行中的子分支默认取消，
    /// 用 --unaffected-scope 声明不受影响的分支
    Revise {
        intent_id: String,
        #[arg(long)]
        outcome: Option<String>,
        #[arg(long = "constraint")]
        constraints: Vec<String>,
        #[arg(long, default_value = "用户修订目标")]
        reason: String,
        #[arg(long = "unaffected-scope")]
        unaffected_scopes: Vec<String>,
    },
    /// 查看与维护分层记忆（list / promote / revoke / correct）
    Memory {
        #[command(subcommand)]
        command: Option<MemoryCommand>,
        /// list：按项目作用域过滤
        #[arg(long)]
        project: Option<String>,
    },
    /// 通过 Runtime Tool Gateway 执行受控工具（Opaque Harness 内部使用）
    ToolCall {
        #[arg(long)]
        tool: String,
        #[arg(long)]
        input: String,
        #[arg(long)]
        idempotency_key: Option<String>,
    },
    /// 启动 loopback HTTP daemon，供 Apple/Web/远程 CLI 共用
    Serve {
        #[arg(long, default_value = "127.0.0.1:4180")]
        bind: String,
        /// 固定 Bearer Token；缺省生成并写入 daemon-token。
        #[arg(long)]
        token: Option<String>,
    },
    /// 注册并同步只读日历连接器
    Calendar {
        intent_id: String,
        /// 本地 ICS 文件或 HTTPS URL
        source: String,
        #[arg(long, default_value = "calendar")]
        name: String,
    },
    /// 注册并同步只读 Linear Connector
    Linear {
        intent_id: String,
        /// 本地 JSON/GraphQL 响应或 HTTPS URL
        source: String,
        #[arg(long, default_value = "linear")]
        name: String,
    },
    /// 通过 Adapter Registry 注册并同步任意 Connector
    Connector {
        intent_id: String,
        /// Adapter ID，例如 notion.api / linear.graphql / calendar.ics
        adapter: String,
        /// 本地文件或 HTTPS URL
        source: String,
        #[arg(long, default_value = "connector")]
        name: String,
    },
    /// 配置 Connector 候选的低风险自动接受边界
    ConnectorPolicy {
        intent_id: String,
        #[arg(long)]
        auto_accept: bool,
        #[arg(long = "allow-origin")]
        allowed_origins: Vec<String>,
        #[arg(long, default_value_t = 80)]
        min_confidence: u8,
        #[arg(long, default_value = "read_only", value_parser = ["read_only", "reversible", "compensatable"])]
        max_effect: String,
    },
    /// 查看已注册连接器
    Connectors { intent_id: String },
    /// 列出 Runtime 已注册的 Connector Adapter 与能力
    ConnectorAdapters,
    /// 查看标准化 World Signal 及失效状态
    Signals { intent_id: String },
    /// 跨 Intent 按事件水位读取 Signal
    SignalFeed {
        #[arg(long, default_value_t = 0)]
        cursor: u64,
        #[arg(long)]
        provider: Option<String>,
        #[arg(long)]
        entity: Option<String>,
    },
    /// 跨 Intent 按事件水位读取 Intent Candidate
    CandidateFeed {
        #[arg(long, default_value_t = 0)]
        cursor: u64,
    },
    /// 运行确定性 Resolver，根据 Signal 提出 Intent Candidate
    ResolveSignals { intent_id: String },
    /// 查看 Intent Candidate
    Candidates { intent_id: String },
    /// 手工提出 Intent Candidate；只提案，不自动创建 Intent
    CandidatePropose {
        intent_id: String,
        outcome: String,
        #[arg(long, default_value = "inferred_need", value_parser = ["user_direct", "external_request", "commitment", "inferred_need", "agent_proposal", "system_recovery"])]
        origin: String,
        #[arg(long = "signal")]
        signals: Vec<String>,
        #[arg(long = "done-when")]
        done_when: Vec<String>,
        #[arg(long = "constraint")]
        constraints: Vec<String>,
        #[arg(long, default_value = "read_only", value_parser = ["read_only", "reversible", "compensatable", "irreversible", "unknown"])]
        max_effect: String,
        #[arg(long, default_value_t = 50)]
        confidence: u8,
        #[arg(long)]
        deadline: Option<String>,
        #[arg(long)]
        dedupe_key: Option<String>,
    },
    /// 接受 Candidate，创建带完整来源血缘的 Intent
    CandidateAccept {
        intent_id: String,
        candidate_id: String,
    },
    /// 拒绝 Candidate，不创建 Intent
    CandidateReject {
        intent_id: String,
        candidate_id: String,
    },
}

#[derive(Subcommand)]
enum MemoryCommand {
    /// 列出记忆（默认行为）
    List,
    /// 用户确认晋升候选到指定作用域（global / project:P / intent:ID）
    Promote {
        candidate_id: String,
        #[arg(long)]
        scope: Option<String>,
    },
    /// 撤销一条记忆
    Revoke {
        memory_id: String,
        #[arg(long, default_value = "用户撤销")]
        reason: String,
    },
    /// 项目级纠正（UserStated）：只进入指定项目作用域，不泛化到全局
    Correct {
        #[arg(long)]
        project: String,
        #[arg(long)]
        subject: String,
        #[arg(long, default_value = "user_correction")]
        predicate: String,
        #[arg(long)]
        value: String,
    },
}

fn main() {
    let cli = Cli::parse();
    if let Err(e) = dispatch(cli) {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

fn dispatch(cli: Cli) -> else_api::Result<()> {
    if let Command::ToolCall {
        tool,
        input,
        idempotency_key,
    } = &cli.command
    {
        run_tool_call(tool, input, idempotency_key.as_deref())?;
        return Ok(());
    }
    if let Command::Serve { bind, token } = &cli.command {
        let daemon = else_daemon::Daemon::start(else_daemon::DaemonConfig {
            dir: cli.dir.clone(),
            bind: bind.clone(),
            token: token.clone(),
        })
        .map_err(|error| else_api::ApiError::NotFound(error.to_string()))?;
        println!("runtime: {}", daemon.url);
        println!("token: {}", daemon.token);
        loop {
            std::thread::park_timeout(std::time::Duration::from_secs(60));
        }
    }
    let runtime = Runtime::open(&cli.dir)?;
    match cli.command {
        Command::Intent {
            outcome,
            project,
            harness,
            allow_opaque,
            opaque_network,
            tool_gateway,
            opaque_max_effect,
        } => {
            let spec = runtime.create_intent(&outcome, project)?;
            let intent_id = spec.id.clone();
            println!("intent: {intent_id}");
            if allow_opaque {
                runtime.configure_opaque_harness(
                    &intent_id,
                    else_protocol::intent::OpaqueHarnessPolicy {
                        enabled: true,
                        allow_native_provider_network: opaque_network,
                        use_tool_gateway: tool_gateway,
                        max_observed_effect: parse_effect(&opaque_max_effect),
                    },
                )?;
            }
            let projection = runtime.advance(&intent_id, Some(&harness))?;
            print_projection(&projection);
        }
        Command::Advance { intent_id, harness } => {
            let projection = runtime.advance(&IntentId::from(intent_id), Some(&harness))?;
            print_projection(&projection);
        }
        Command::Drive {
            intent_id,
            harness,
            max_steps,
        } => {
            let driven = runtime.drive(&IntentId::from(intent_id), Some(&harness), max_steps)?;
            println!("stopped: {:?}", driven.stopped);
            println!("steps: {}", driven.steps);
            print_projection(&driven.projection);
        }
        Command::Event {
            intent_id,
            source,
            summary,
        } => {
            let projection =
                runtime.record_external_event(&IntentId::from(intent_id), &source, &summary)?;
            print_projection(&projection);
        }
        Command::WaitExternal { intent_id, reason } => {
            let projection = runtime.wait_for_external(&IntentId::from(intent_id), &reason)?;
            print_projection(&projection);
        }
        Command::Wake {
            intent_id,
            seconds,
            reason,
        } => {
            let timer_id = runtime.schedule_timer(
                &IntentId::from(intent_id),
                chrono::Utc::now() + chrono::Duration::seconds(seconds),
                &reason,
            )?;
            println!("timer: {timer_id}");
        }
        Command::Run { intent_id, harness } => {
            let intent_id = IntentId::from(intent_id);
            let projection = if harness == else_harness::SCRIPTED {
                runtime.run(&intent_id)?
            } else {
                let adapters = else_harness::builtin_adapters();
                let Some(adapter) = adapters.iter().find(|a| a.name() == harness) else {
                    let available = adapters
                        .iter()
                        .map(|a| a.name())
                        .collect::<Vec<_>>()
                        .join(", ");
                    return Err(else_api::ApiError::NotFound(format!(
                        "harness {harness}（可用：scripted{available}）",
                        available = if available.is_empty() {
                            String::new()
                        } else {
                            format!(", {available}")
                        }
                    )));
                };
                runtime.run_adapter(&intent_id, adapter.as_ref())?
            };
            print_projection(&projection);
        }
        Command::Harnesses => {
            println!("scripted（内置确定性执行器）");
            for adapter in else_harness::builtin_adapters() {
                let caps = adapter.capabilities();
                println!(
                    "{}（cancel={:?} usage={:?} tool_effects={:?} network={:?}）",
                    adapter.name(),
                    caps.cancel,
                    caps.usage,
                    caps.tool_effects,
                    adapter.network_access()
                );
            }
        }
        Command::AllowOpaque {
            intent_id,
            network,
            tool_gateway,
            max_effect,
        } => {
            let spec = runtime.configure_opaque_harness(
                &IntentId::from(intent_id),
                else_protocol::intent::OpaqueHarnessPolicy {
                    enabled: true,
                    allow_native_provider_network: network,
                    use_tool_gateway: tool_gateway,
                    max_observed_effect: parse_effect(&max_effect),
                },
            )?;
            println!(
                "opaque: enabled network={} max_effect={:?} revision={}",
                spec.opaque_harness_policy.allow_native_provider_network,
                spec.opaque_harness_policy.max_observed_effect,
                spec.revision
            );
        }
        Command::Abort { intent_id, reason } => {
            let projection = runtime.abort(
                &IntentId::from(intent_id),
                reason.as_deref().unwrap_or("用户取消"),
            )?;
            print_projection(&projection);
        }
        Command::List => {
            for spec in runtime.list_intents()? {
                println!("{}  {}", spec.id, spec.outcome);
            }
        }
        Command::Scopes { intent_id } => {
            let projection = runtime.projection(&IntentId::from(intent_id))?;
            for (id, scope) in projection.scopes {
                println!(
                    "scope {id} parent={:?} replaces={:?} state={:?} generation={} failures={}",
                    scope.parent,
                    scope.replaces,
                    scope.state,
                    scope.generation,
                    scope.failure_count
                );
                if let Some(failure) = scope.last_failure {
                    println!(
                        "  last_failure: {:?} {} attempt={}: {}",
                        failure.class, failure.code, failure.attempt, failure.message
                    );
                }
            }
        }
        Command::Delegate {
            intent_id,
            parent_scope,
            task,
            depends_on,
            max_effect,
        } => {
            use else_protocol::effect::SideEffectKind;
            let max_effect = match max_effect.as_str() {
                "read_only" => SideEffectKind::ReadOnly,
                "reversible" => SideEffectKind::Reversible,
                "compensatable" => SideEffectKind::Compensatable,
                "irreversible" => SideEffectKind::Irreversible,
                _ => SideEffectKind::Unknown,
            };
            let depends_on: Vec<_> = depends_on
                .iter()
                .map(|scope| else_protocol::ids::ScopeId::from(scope.as_str()))
                .collect();
            let child = runtime.spawn_subagent_with_dependencies(
                &IntentId::from(intent_id),
                &else_protocol::ids::ScopeId::from(parent_scope),
                &task,
                max_effect,
                depends_on,
            )?;
            println!("child scope: {child}");
        }
        Command::RunScope {
            intent_id,
            scope_id,
            harness,
        } => {
            let projection = runtime.dispatch_scope(
                &IntentId::from(intent_id),
                &else_protocol::ids::ScopeId::from(scope_id),
                Some(&harness),
            )?;
            print_projection(&projection);
        }
        Command::Decisions { intent_id } => {
            let projection = runtime.projection(&IntentId::from(intent_id))?;
            print_decisions(&projection);
        }
        Command::Attention { intent_id } => {
            let projection = runtime.projection(&IntentId::from(intent_id))?;
            print_attention(&projection);
        }
        Command::Plan { intent_id } => {
            let projection = runtime.projection(&IntentId::from(intent_id))?;
            print_plan(&projection);
        }
        Command::Decide {
            intent_id,
            request_id,
            choice,
        } => {
            let choice = match choice.as_str() {
                "approve" | "reject" => DecisionChoice::Option(choice),
                other => DecisionChoice::FreeText(other.to_string()),
            };
            let projection = runtime.decide(
                &IntentId::from(intent_id),
                &AttentionId::from(request_id),
                choice,
            )?;
            print_projection(&projection);
        }
        Command::Log { intent_id } => {
            for event in runtime.events(&IntentId::from(intent_id))? {
                println!(
                    "#{:<4} {}",
                    event.seq,
                    serde_json::to_string(&event.envelope.payload)
                        .unwrap_or_else(|_| "<codec error>".into())
                );
            }
        }
        Command::Resources { intent_id } => {
            let report = runtime.resources(&IntentId::from(intent_id))?;
            println!(
                "model: tokens={}/{} spend_usd_micros={}/{}",
                report.usage.model_tokens,
                report.budget.max_model_tokens,
                report.usage.model_spend_usd_micros,
                report.budget.max_model_spend_usd_micros
            );
            println!(
                "tools: calls={}/{} gateway={} observed_opaque={}",
                report.usage.tool_calls,
                report.budget.max_tool_calls,
                report
                    .usage
                    .tool_calls
                    .saturating_sub(report.usage.opaque_tool_calls),
                report.usage.opaque_tool_calls
            );
            println!(
                "sandbox: actual_seconds={} reserved_seconds={} committed={}/{}",
                report.usage.sandbox_seconds,
                report.usage.sandbox_reserved_seconds,
                report.usage.sandbox_committed_seconds(),
                report.budget.max_sandbox_seconds
            );
            println!(
                "sandbox evidence: reported={}s estimated={}s unknown={}s actual_millis={}",
                report.usage.sandbox_reported_seconds,
                report.usage.sandbox_estimated_seconds,
                report.usage.sandbox_unknown_seconds,
                report.usage.sandbox_actual_millis
            );
            for sandbox in report.sandboxes {
                println!(
                    "sandbox {} backend={} started={} finished={} elapsed_ms={} basis={:?} termination={:?}",
                    sandbox.lease_id,
                    sandbox.backend,
                    sandbox.started_at.to_rfc3339(),
                    sandbox.finished_at.to_rfc3339(),
                    sandbox.elapsed_millis,
                    sandbox.basis,
                    sandbox.termination
                );
            }
        }
        Command::Checkpoint { intent_id } => {
            let checkpoint = runtime.checkpoint(&IntentId::from(intent_id))?;
            println!("checkpoint: {}", checkpoint.id);
            println!("event_cursor: {}", checkpoint.event_cursor);
            println!("inflight_effects: {}", checkpoint.inflight.len());
        }
        Command::Rerun {
            intent_id,
            checkpoint_id,
            alternative,
        } => {
            let intent_id = IntentId::from(intent_id);
            let (scope_id, generation) = runtime.rerun(
                &intent_id,
                &else_protocol::ids::CheckpointId::from(checkpoint_id),
                alternative,
            )?;
            println!("branch scope: {scope_id}");
            println!("generation: {generation}");
            let projection = runtime.projection(&intent_id)?;
            print_projection(&projection);
        }
        Command::Compare { intent_id } => {
            print!("{}", runtime.compare(&IntentId::from(intent_id))?);
        }
        Command::Revise {
            intent_id,
            outcome,
            constraints,
            reason,
            unaffected_scopes,
        } => {
            let unaffected: Vec<_> = unaffected_scopes
                .iter()
                .map(|scope| else_protocol::ids::ScopeId::from(scope.as_str()))
                .collect();
            let spec = runtime.revise_intent(
                &IntentId::from(intent_id),
                else_api::IntentRevision {
                    outcome,
                    add_constraints: constraints,
                    reason,
                },
                &unaffected,
            )?;
            println!("revision: {}", spec.revision);
            println!("outcome: {}", spec.outcome);
        }
        Command::Memory { command, project } => {
            use else_protocol::memory::{MemoryScope, MemoryTier};
            let parse_scope = |raw: &str| -> Result<MemoryScope, String> {
                if raw == "global" {
                    Ok(MemoryScope::Global)
                } else if let Some(name) = raw.strip_prefix("project:") {
                    Ok(MemoryScope::Project(name.to_string()))
                } else if let Some(id) = raw.strip_prefix("intent:") {
                    Ok(MemoryScope::Intent(IntentId::from(id)))
                } else {
                    Err(format!(
                        "unknown scope {raw} (global / project:P / intent:ID)"
                    ))
                }
            };
            match command {
                None | Some(MemoryCommand::List) => {
                    let memory = runtime.memory_projection()?;
                    let tiers = [
                        MemoryTier::Working,
                        MemoryTier::Episodic,
                        MemoryTier::Semantic,
                    ];
                    let recalled =
                        memory.recall(project.as_deref(), None, &tiers, chrono::Utc::now());
                    println!("promoted (scope={:?}):", project.as_deref().unwrap_or("*"));
                    for record in &recalled {
                        println!(
                            "  {} {:?} {:?} {} = {}",
                            record.id, record.tier, record.scope, record.subject, record.value
                        );
                    }
                    if project.is_none() {
                        println!("candidates:");
                        for record in memory.candidates.values() {
                            println!(
                                "  {} {:?} {:?} {} = {}",
                                record.id, record.tier, record.scope, record.subject, record.value
                            );
                        }
                    }
                }
                Some(MemoryCommand::Promote {
                    candidate_id,
                    scope,
                }) => {
                    let scope = scope
                        .map(|raw| parse_scope(&raw))
                        .transpose()
                        .map_err(else_api::ApiError::NotFound)?;
                    let record = runtime
                        .promote_memory(&else_protocol::ids::MemoryId::from(candidate_id), scope)?;
                    println!("promoted: {} -> {:?}", record.id, record.scope);
                }
                Some(MemoryCommand::Revoke { memory_id, reason }) => {
                    runtime.revoke_memory(
                        &else_protocol::ids::MemoryId::from(memory_id.clone()),
                        &reason,
                    )?;
                    println!("revoked: {memory_id}");
                }
                Some(MemoryCommand::Correct {
                    project,
                    subject,
                    predicate,
                    value,
                }) => {
                    let record = runtime.correct_memory(
                        MemoryScope::Project(project),
                        &subject,
                        &predicate,
                        &value,
                    )?;
                    println!("corrected: {} (project scope only)", record.id);
                }
            }
        }
        Command::ToolCall { .. } => unreachable!("handled before Runtime::open"),
        Command::Serve { .. } => unreachable!("handled before Runtime::open"),
        Command::Calendar {
            intent_id,
            source,
            name,
        } => {
            let intent_id = IntentId::from(intent_id);
            let connector = runtime.register_calendar_connector(&intent_id, &name, &source)?;
            let result = runtime.sync_connector(&intent_id, &connector.id)?;
            println!(
                "connector: {} observed={} inserted={} skipped={} candidates={} auto_accepted={} cursor={}",
                connector.id,
                result.observed,
                result.inserted,
                result.skipped,
                result.candidates_proposed,
                result.auto_accepted,
                result.cursor.as_deref().unwrap_or("none")
            );
        }
        Command::Linear {
            intent_id,
            source,
            name,
        } => {
            let intent_id = IntentId::from(intent_id);
            let connector = runtime.register_linear_connector(&intent_id, &name, &source)?;
            let result = runtime.sync_connector(&intent_id, &connector.id)?;
            println!(
                "connector: {} observed={} inserted={} skipped={} candidates={} auto_accepted={} cursor={} webhook_session={}",
                connector.id,
                result.observed,
                result.inserted,
                result.skipped,
                result.candidates_proposed,
                result.auto_accepted,
                result.cursor.as_deref().unwrap_or("none"),
                connector.webhook_session.as_deref().unwrap_or("none")
            );
        }
        Command::Connector {
            intent_id,
            adapter,
            source,
            name,
        } => {
            let intent_id = IntentId::from(intent_id);
            let connector =
                runtime.register_connector_adapter(&intent_id, &name, &source, &adapter)?;
            let result = runtime.sync_connector(&intent_id, &connector.id)?;
            println!(
                "connector: {} adapter={} observed={} inserted={} skipped={} candidates={} auto_accepted={} cursor={}",
                connector.id,
                connector.adapter,
                result.observed,
                result.inserted,
                result.skipped,
                result.candidates_proposed,
                result.auto_accepted,
                result.cursor.as_deref().unwrap_or("none")
            );
        }
        Command::ConnectorPolicy {
            intent_id,
            auto_accept,
            allowed_origins,
            min_confidence,
            max_effect,
        } => {
            use else_protocol::connector::ConnectorPolicy;
            let policy = ConnectorPolicy {
                auto_accept_low_risk: auto_accept,
                allowed_origins: if allowed_origins.is_empty() {
                    ConnectorPolicy::default().allowed_origins
                } else {
                    allowed_origins
                        .iter()
                        .map(|origin| parse_intent_origin(origin))
                        .collect()
                },
                min_confidence,
                max_autonomous_effect: parse_effect(&max_effect),
            };
            let spec = runtime.configure_connector_policy(&IntentId::from(intent_id), policy)?;
            println!(
                "connector policy: auto_accept={} min_confidence={} max_effect={:?} revision={}",
                spec.connector_policy.auto_accept_low_risk,
                spec.connector_policy.min_confidence,
                spec.connector_policy.max_autonomous_effect,
                spec.revision
            );
        }
        Command::Connectors { intent_id } => {
            let projection = runtime.projection(&IntentId::from(intent_id))?;
            for connector in projection.connectors.values() {
                println!(
                    "{} {:?} enabled={} health={:?} provider={} sync={:?} read={:?} write={:?} source={}",
                    connector.id,
                    connector.adapter,
                    connector.enabled,
                    connector.manifest.health,
                    connector.manifest.provider,
                    connector.manifest.sync_modes,
                    connector.manifest.read_capabilities,
                    connector.manifest.write_capabilities,
                    connector.source
                );
            }
        }
        Command::ConnectorAdapters => {
            for manifest in runtime.connector_manifests() {
                println!(
                    "{} provider={} auth={:?} sync={:?} read={:?} write={:?} health={:?}",
                    manifest.adapter,
                    manifest.provider,
                    manifest.auth_mode,
                    manifest.sync_modes,
                    manifest.read_capabilities,
                    manifest.write_capabilities,
                    manifest.health
                );
            }
        }
        Command::Signals { intent_id } => {
            let projection = runtime.projection(&IntentId::from(intent_id))?;
            for signal in projection.world_signals.values() {
                println!(
                    "{} {:?} {:?} {:?} occurred={} valid_until={} trust={} summary={}",
                    signal.id,
                    signal.object_kind,
                    signal.change,
                    signal.status,
                    signal.occurred_at.to_rfc3339(),
                    signal
                        .valid_until
                        .map(|value| value.to_rfc3339())
                        .unwrap_or_else(|| "none".into()),
                    signal.source_trust,
                    signal.summary
                );
            }
        }
        Command::SignalFeed {
            cursor,
            provider,
            entity,
        } => {
            for record in runtime.world_signals(cursor, provider.as_deref(), entity.as_deref())? {
                println!(
                    "#{} intent={} {} {:?} {:?} {:?} {}",
                    record.seq,
                    record.intent_id,
                    record.signal.id,
                    record.signal.object_kind,
                    record.signal.change,
                    record.signal.status,
                    record.signal.summary
                );
            }
        }
        Command::CandidateFeed { cursor } => {
            for record in runtime.intent_candidates(cursor)? {
                println!(
                    "#{} intent={} {} {:?} {:?} {}",
                    record.seq,
                    record.intent_id,
                    record.candidate.id,
                    record.candidate.origin,
                    record.candidate.state,
                    record.candidate.proposed_outcome
                );
            }
        }
        Command::ResolveSignals { intent_id } => {
            let candidates = runtime.resolve_signals(&IntentId::from(intent_id))?;
            if candidates.is_empty() {
                println!("candidates: none");
            }
            for candidate in candidates {
                print_candidate(&candidate);
            }
        }
        Command::Candidates { intent_id } => {
            let projection = runtime.projection(&IntentId::from(intent_id))?;
            if projection.intent_candidates.is_empty() {
                println!("candidates: none");
            }
            for candidate in projection.intent_candidates.values() {
                print_candidate(candidate);
            }
        }
        Command::CandidatePropose {
            intent_id,
            outcome,
            origin,
            signals,
            done_when,
            constraints,
            max_effect,
            confidence,
            deadline,
            dedupe_key,
        } => {
            use else_protocol::connector::{IntentCandidate, IntentCandidateState};
            let intent_id = IntentId::from(intent_id);
            let origin = parse_intent_origin(&origin);
            let related_signal_ids = signals
                .into_iter()
                .map(else_protocol::ids::WorldSignalId::from)
                .collect::<Vec<_>>();
            let done_when = if done_when.is_empty() {
                vec![else_protocol::intent::Criterion::manual(format!(
                    "确认完成：{outcome}"
                ))]
            } else {
                done_when
                    .into_iter()
                    .map(else_protocol::intent::Criterion::manual)
                    .collect()
            };
            let deadline = deadline
                .map(|value| {
                    chrono::DateTime::parse_from_rfc3339(&value)
                        .map(|value| value.with_timezone(&chrono::Utc))
                        .map_err(|error| {
                            else_api::ApiError::NotFound(format!(
                                "invalid deadline RFC3339: {error}"
                            ))
                        })
                })
                .transpose()?;
            let dedupe_key = dedupe_key.unwrap_or_else(|| {
                let signals = related_signal_ids
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(",");
                format!("manual-candidate:{:016x}", stable_hash(&outcome, &signals))
            });
            let candidate = runtime.propose_intent_candidate(
                &intent_id,
                IntentCandidate {
                    id: else_protocol::ids::IntentCandidateId::new(),
                    origin,
                    proposed_outcome: outcome,
                    done_when,
                    constraints,
                    authority: else_protocol::intent::Authority {
                        max_autonomous_effect: parse_effect(&max_effect),
                        forbidden: vec![],
                    },
                    attention_policy: else_protocol::intent::AttentionPolicy {
                        allow_interrupt: false,
                        batch_interval_minutes: 60,
                    },
                    deadline,
                    related_signal_ids,
                    related_intent_id: Some(intent_id.clone()),
                    confidence,
                    dedupe_key,
                    state: IntentCandidateState::Proposed,
                    accepted_intent_id: None,
                    accepted_by: None,
                },
            )?;
            print_candidate(&candidate);
        }
        Command::CandidateAccept {
            intent_id,
            candidate_id,
        } => {
            let spec = runtime
                .resolve_intent_candidate(
                    &IntentId::from(intent_id),
                    &else_protocol::ids::IntentCandidateId::from(candidate_id),
                    true,
                )?
                .ok_or_else(|| {
                    else_api::ApiError::NotFound("candidate acceptance produced no intent".into())
                })?;
            println!("intent: {}", spec.id);
            println!("outcome: {}", spec.outcome);
            if let Some(origin) = spec.origin {
                println!(
                    "origin: {:?} candidate={:?} signals={} connectors={}",
                    origin.kind,
                    origin.candidate_id,
                    origin.signal_ids.len(),
                    origin.connector_ids.len()
                );
            }
        }
        Command::CandidateReject {
            intent_id,
            candidate_id,
        } => {
            runtime.resolve_intent_candidate(
                &IntentId::from(intent_id),
                &else_protocol::ids::IntentCandidateId::from(candidate_id),
                false,
            )?;
            println!("rejected");
        }
    }
    Ok(())
}

fn print_candidate(candidate: &else_protocol::connector::IntentCandidate) {
    println!(
        "{} {:?} {:?} confidence={} signals={} outcome={}",
        candidate.id,
        candidate.origin,
        candidate.state,
        candidate.confidence,
        candidate.related_signal_ids.len(),
        candidate.proposed_outcome
    );
}

fn parse_intent_origin(origin: &str) -> else_protocol::connector::IntentOriginKind {
    use else_protocol::connector::IntentOriginKind;
    match origin {
        "user_direct" => IntentOriginKind::UserDirect,
        "external_request" => IntentOriginKind::ExternalRequest,
        "commitment" => IntentOriginKind::Commitment,
        "agent_proposal" => IntentOriginKind::AgentProposal,
        "system_recovery" => IntentOriginKind::SystemRecovery,
        _ => IntentOriginKind::InferredNeed,
    }
}

fn run_tool_call(tool: &str, input: &str, idempotency_key: Option<&str>) -> else_api::Result<()> {
    use std::io::{Read, Write};
    use std::net::TcpStream;

    let url = std::env::var("ELSE_TOOL_URL")
        .map_err(|_| else_api::ApiError::NotFound("ELSE_TOOL_URL is not set".into()))?;
    let token = std::env::var("ELSE_TOOL_TOKEN")
        .map_err(|_| else_api::ApiError::NotFound("ELSE_TOOL_TOKEN is not set".into()))?;
    let authority = url.strip_prefix("http://").ok_or_else(|| {
        else_api::ApiError::NotFound("ELSE_TOOL_URL must be http loopback".into())
    })?;
    let input: serde_json::Value = serde_json::from_str(input).map_err(|error| {
        else_api::ApiError::NotFound(format!("invalid tool input JSON: {error}"))
    })?;
    let idempotency_key = idempotency_key.map(str::to_string).unwrap_or_else(|| {
        format!(
            "auto-{:016x}",
            stable_hash(tool, input.to_string().as_str())
        )
    });
    let body = serde_json::json!({
        "tool": tool,
        "input": input,
        "idempotency_key": idempotency_key,
    })
    .to_string();
    let mut stream = TcpStream::connect(authority)
        .map_err(|error| else_api::ApiError::NotFound(format!("tool gateway: {error}")))?;
    write!(
        stream,
        "POST /tool HTTP/1.1\r\nHost: {authority}\r\nAuthorization: Bearer {token}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )?;
    let mut response = String::new();
    stream.read_to_string(&mut response)?;
    let success = response.starts_with("HTTP/1.1 200");
    let body = response
        .split_once("\r\n\r\n")
        .map(|(_, body)| body)
        .unwrap_or(&response);
    println!("{body}");
    if success {
        Ok(())
    } else {
        Err(else_api::ApiError::NotFound(body.into()))
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

fn print_projection(projection: &else_ledger::IntentProjection) {
    println!("state: {:?}", projection.state);
    println!("artifacts: {}", projection.artifacts.len());
    print_decisions(projection);
}

fn print_decisions(projection: &else_ledger::IntentProjection) {
    let now = chrono::Utc::now();
    let windows = projection.actionable_attention_windows(now);
    let mut grouped = BTreeSet::new();
    for (window, requests) in windows {
        for request in &requests {
            grouped.insert(request.id.clone());
        }
        let label = match window.kind {
            else_protocol::attention::AttentionWindowKind::Batch => "batch",
            else_protocol::attention::AttentionWindowKind::Defer => "deferred",
        };
        println!(
            "decision window {} ({label}, {} items):",
            window.id,
            requests.len()
        );
        for request in requests {
            print_decision_request(request);
        }
    }
    let requests = projection.actionable_requests(chrono::Utc::now());
    let ungrouped: Vec<_> = requests
        .into_iter()
        .filter(|request| !grouped.contains(&request.id))
        .collect();
    if ungrouped.is_empty() && grouped.is_empty() {
        println!("decisions: none");
        return;
    }
    for request in ungrouped {
        print_decision_request(request);
    }
}

fn print_decision_request(request: &else_protocol::attention::AttentionRequest) {
    let id = &request.id;
    println!("decision {id}: {}", request.summary);
    for option in &request.options {
        println!("  [{}] {}", option.id, option.label);
    }
}

fn print_attention(projection: &else_ledger::IntentProjection) {
    let now = chrono::Utc::now();
    let metrics = projection.attention_metrics(now);
    println!(
        "attention: requested={} interventions={} resolved={}",
        metrics.requested, metrics.interventions, metrics.resolved
    );
    println!(
        "signals: interrupt={} batch={} defer={} silent={}",
        metrics.interrupt_notifications,
        metrics.batch_notifications,
        metrics.defer_notifications,
        metrics.silent_requests
    );
    println!(
        "quality: batch_rate={} misinterrupt_rate={} missed_critical={}",
        format_rate(metrics.batch_rate),
        format_rate(metrics.misinterrupt_rate),
        metrics.missed_critical
    );
    if projection.attention_windows.is_empty() {
        println!("windows: none");
        return;
    }
    for window in projection.attention_windows.values() {
        println!(
            "window {} kind={:?} members={} due={} closed={}",
            window.id,
            window.kind,
            window.request_ids.len(),
            window.due_at.to_rfc3339(),
            window
                .closed_at
                .map(|at| at.to_rfc3339())
                .unwrap_or_else(|| "open".into())
        );
    }
}

fn format_rate(rate: Option<f64>) -> String {
    match rate {
        Some(rate) => format!("{:.1}%", rate * 100.0),
        None => "n/a".into(),
    }
}

fn parse_effect(raw: &str) -> else_protocol::effect::SideEffectKind {
    use else_protocol::effect::SideEffectKind;
    match raw {
        "read_only" => SideEffectKind::ReadOnly,
        "reversible" => SideEffectKind::Reversible,
        "compensatable" => SideEffectKind::Compensatable,
        "irreversible" => SideEffectKind::Irreversible,
        _ => SideEffectKind::Unknown,
    }
}

fn print_plan(projection: &else_ledger::IntentProjection) {
    let Some(plan) = projection.current_plan() else {
        println!("plan: none");
        return;
    };
    println!(
        "plan {} revision={} intent_revision={} supersedes={:?}",
        plan.id, plan.revision, plan.intent_revision, plan.supersedes
    );
    println!("summary: {}", plan.summary);
    for step in &plan.steps {
        println!(
            "step {} status={:?} scope={:?}: {}",
            step.id, step.status, step.scope_id, step.title
        );
        if !step.watch_sources.is_empty() || !step.watch_terms.is_empty() {
            println!(
                "  watches: sources={:?} terms={:?}",
                step.watch_sources, step.watch_terms
            );
        }
    }
    for evidence in &plan.evidence {
        println!(
            "evidence {:?} {}: {}",
            evidence.kind,
            evidence.event_id.as_deref().unwrap_or("-"),
            evidence.summary
        );
    }
    if !projection.plan_impacts.is_empty() {
        println!("impacts:");
        for impact in &projection.plan_impacts {
            println!(
                "  {} -> affected={} scopes={} unaffected={}",
                impact.source,
                impact.affected_step_ids.len(),
                impact.invalidated_scope_ids.len(),
                impact.unaffected_step_ids.len()
            );
            println!("  {}", impact.explanation);
        }
    }
}
