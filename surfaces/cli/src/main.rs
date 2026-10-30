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
    let runtime = Runtime::open(&cli.dir)?;
    match cli.command {
        Command::Intent {
            outcome,
            project,
            harness,
        } => {
            let spec = runtime.create_intent(&outcome, project)?;
            let intent_id = spec.id.clone();
            println!("intent: {intent_id}");
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
                    "{}（cancel={:?} usage={:?} tool_effects={:?}）",
                    adapter.name(),
                    caps.cancel,
                    caps.usage,
                    caps.tool_effects
                );
            }
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
                    "scope {id} parent={:?} state={:?} generation={}",
                    scope.parent, scope.state, scope.generation
                );
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
                "tools: calls={}/{}",
                report.usage.tool_calls, report.budget.max_tool_calls
            );
            println!(
                "sandbox: seconds={}/{}",
                report.usage.sandbox_seconds, report.budget.max_sandbox_seconds
            );
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
    }
    Ok(())
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
