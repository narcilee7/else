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
    /// 注册真实 Google Calendar API Connector，并支持事件更新/核验/补偿
    GoogleCalendar {
        intent_id: String,
        #[arg(long, default_value = "primary")]
        calendar_id: String,
        #[arg(long, conflicts_with = "token_keychain_service")]
        token_env: Option<String>,
        #[arg(long, conflicts_with = "token_env")]
        token_keychain_service: Option<String>,
        #[arg(long)]
        refresh_token_env: Option<String>,
        #[arg(long)]
        token_url: Option<String>,
        #[arg(long)]
        client_id: Option<String>,
        #[arg(long)]
        client_secret_env: Option<String>,
        #[arg(long)]
        expires_at: Option<String>,
        #[arg(long, default_value_t = 300)]
        sync_interval: u64,
        #[arg(long, default_value = "google-calendar")]
        name: String,
    },
    /// 注册并同步只读 Linear Connector
    Linear {
        intent_id: String,
        /// 本地 JSON/GraphQL 响应或 HTTPS URL
        source: String,
        /// 从环境变量读取 API Token；秘密不进入命令行历史
        #[arg(long)]
        token_env: Option<String>,
        /// 可选 OAuth refresh token 的环境变量
        #[arg(long)]
        refresh_token_env: Option<String>,
        #[arg(long)]
        token_url: Option<String>,
        #[arg(long)]
        client_id: Option<String>,
        #[arg(long)]
        client_secret_env: Option<String>,
        #[arg(long)]
        expires_at: Option<String>,
        #[arg(long = "scope")]
        credential_scopes: Vec<String>,
        /// 后台同步间隔（秒）
        #[arg(long, default_value_t = 300)]
        sync_interval: u64,
        #[arg(long, default_value = "linear")]
        name: String,
    },
    /// 注册真实 Notion Search / Database Connector 并执行首次 Backfill
    Notion {
        intent_id: String,
        /// 可选 Database ID 或 Database URL；省略时同步所有授权页面/数据库
        #[arg(long)]
        database: Option<String>,
        /// 保存 Notion Integration Token 的环境变量
        #[arg(long, conflicts_with = "token_keychain_service")]
        token_env: Option<String>,
        /// 从 macOS Keychain service 读取 Notion Integration Token
        #[arg(long, conflicts_with = "token_env")]
        token_keychain_service: Option<String>,
        /// 后台同步间隔（秒）
        #[arg(long, default_value_t = 300)]
        sync_interval: u64,
        #[arg(long, default_value = "notion")]
        name: String,
    },
    /// 通过 Adapter Registry 注册并同步任意 Connector
    Connector {
        intent_id: String,
        /// Adapter ID，例如 notion.api / linear.graphql / calendar.ics
        adapter: String,
        /// 本地文件或 HTTPS URL
        source: String,
        #[arg(long)]
        token_env: Option<String>,
        #[arg(long)]
        refresh_token_env: Option<String>,
        #[arg(long)]
        token_url: Option<String>,
        #[arg(long)]
        client_id: Option<String>,
        #[arg(long)]
        client_secret_env: Option<String>,
        #[arg(long)]
        expires_at: Option<String>,
        #[arg(long = "scope")]
        credential_scopes: Vec<String>,
        #[arg(long, default_value_t = 300)]
        sync_interval: u64,
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
    /// 提案一次受 Gateway 管控的外部写动作
    ConnectorActionPropose {
        intent_id: String,
        connector_id: String,
        capability: String,
        target: String,
        #[arg(long, default_value = "{}")]
        input: String,
        #[arg(long)]
        idempotency_key: Option<String>,
    },
    /// 执行外部写动作；超出授权时返回 Decision Request
    ConnectorActionExecute {
        intent_id: String,
        action_id: String,
    },
    /// 回源核验 Executing / Unknown / Applied 动作
    ConnectorActionVerify {
        intent_id: String,
        action_id: String,
    },
    /// 通过 Gateway 与 Decision 补偿已应用动作
    ConnectorActionCompensate {
        intent_id: String,
        action_id: String,
    },
    /// 查看外部写动作状态与最终结论
    ConnectorActions { intent_id: String },
    /// 撤销 Connector 私有凭据；后续同步失败关闭
    ConnectorCredentialRevoke {
        intent_id: String,
        connector_id: String,
    },
    /// 更新 Connector 私有凭据，不重复注册 Connector
    ConnectorCredentialSet {
        intent_id: String,
        connector_id: String,
        #[arg(long, conflicts_with = "token_keychain_service")]
        token_env: Option<String>,
        #[arg(long, conflicts_with = "token_env")]
        token_keychain_service: Option<String>,
    },
    /// 列出 Runtime 已注册的 Connector Adapter 与能力
    ConnectorAdapters,
    /// 查看标准化 World Signal 及失效状态
    Signals { intent_id: String },
    /// 查看承诺、等待外部、行动项与风险聚合出的 Personal World
    World {
        intent_id: String,
        #[arg(long)]
        include_resolved: bool,
    },
    /// 将 Personal World 条目标记完成或忽略
    WorldResolve {
        intent_id: String,
        item_id: String,
        #[arg(value_parser = ["completed", "dismissed"])]
        state: String,
        #[arg(long)]
        note: Option<String>,
    },
    /// 跨 Intent 生成 Personal Brief
    Brief,
    /// 按规范化 Project Key 查看跨 Provider World
    Projects {
        intent_id: Option<String>,
        #[arg(long)]
        include_resolved: bool,
    },
    /// 按 Contact / Customer / Thread / Milestone 查看跨 Provider 关系
    Identities {
        intent_id: Option<String>,
        #[arg(long)]
        include_resolved: bool,
    },
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
                "connector: {} observed={} inserted={} skipped={} candidates={} auto_accepted={} pages={} backfill_complete={} cursor={}",
                connector.id,
                result.observed,
                result.inserted,
                result.skipped,
                result.candidates_proposed,
                result.auto_accepted,
                result.pages_synced,
                result.backfill_complete,
                result.cursor.as_deref().unwrap_or("none")
            );
        }
        Command::GoogleCalendar {
            intent_id,
            calendar_id,
            token_env,
            token_keychain_service,
            refresh_token_env,
            token_url,
            client_id,
            client_secret_env,
            expires_at,
            sync_interval,
            name,
        } => {
            let intent_id = IntentId::from(intent_id);
            let source = google_calendar_source_url(&calendar_id)?;
            let credential = if let Some(service) = token_keychain_service {
                Some(connector_credential_from_access_token(
                    &runtime,
                    "calendar.google",
                    read_keychain_secret(&service)?,
                )?)
            } else {
                build_connector_credential(
                    &runtime,
                    "calendar.google",
                    Some(token_env.unwrap_or_else(|| "GOOGLE_CALENDAR_TOKEN".into())),
                    refresh_token_env,
                    token_url,
                    client_id,
                    client_secret_env,
                    expires_at,
                    vec![],
                )?
            };
            let connector = runtime.register_connector_adapter_with_credential(
                &intent_id,
                &name,
                &source,
                "calendar.google",
                credential,
                sync_interval,
            )?;
            let result = runtime.sync_connector(&intent_id, &connector.id)?;
            println!(
                "connector: {} adapter=calendar.google source={} observed={} inserted={} skipped={} candidates={} pages={} backfill_complete={} cursor={}",
                connector.id,
                connector.source,
                result.observed,
                result.inserted,
                result.skipped,
                result.candidates_proposed,
                result.pages_synced,
                result.backfill_complete,
                result.cursor.as_deref().unwrap_or("none")
            );
        }
        Command::Linear {
            intent_id,
            source,
            token_env,
            refresh_token_env,
            token_url,
            client_id,
            client_secret_env,
            expires_at,
            credential_scopes,
            sync_interval,
            name,
        } => {
            let intent_id = IntentId::from(intent_id);
            let credential = build_connector_credential(
                &runtime,
                "linear.graphql",
                token_env,
                refresh_token_env,
                token_url,
                client_id,
                client_secret_env,
                expires_at,
                credential_scopes,
            )?;
            let connector = runtime.register_connector_adapter_with_credential(
                &intent_id,
                &name,
                &source,
                "linear.graphql",
                credential,
                sync_interval,
            )?;
            let result = runtime.sync_connector(&intent_id, &connector.id)?;
            println!(
                "connector: {} observed={} inserted={} skipped={} candidates={} auto_accepted={} pages={} backfill_complete={} cursor={} webhook_session={} credential={}",
                connector.id,
                result.observed,
                result.inserted,
                result.skipped,
                result.candidates_proposed,
                result.auto_accepted,
                result.pages_synced,
                result.backfill_complete,
                result.cursor.as_deref().unwrap_or("none"),
                connector.webhook_session.as_deref().unwrap_or("none"),
                connector.credential.is_some()
            );
        }
        Command::Notion {
            intent_id,
            database,
            token_env,
            token_keychain_service,
            sync_interval,
            name,
        } => {
            let intent_id = IntentId::from(intent_id);
            let source = notion_source_url(database.as_deref())?;
            let access_token = if let Some(service) = token_keychain_service {
                read_keychain_secret(&service)?
            } else {
                read_environment_secret(token_env.as_deref().unwrap_or("NOTION_TOKEN"))?
            };
            let credential =
                connector_credential_from_access_token(&runtime, "notion.api", access_token)?;
            let connector = runtime.register_connector_adapter_with_credential(
                &intent_id,
                &name,
                &source,
                "notion.api",
                Some(credential),
                sync_interval,
            )?;
            let result = runtime.sync_connector(&intent_id, &connector.id)?;
            println!(
                "connector: {} adapter=notion.api observed={} inserted={} skipped={} candidates={} auto_accepted={} pages={} backfill_complete={} cursor={} credential={}",
                connector.id,
                result.observed,
                result.inserted,
                result.skipped,
                result.candidates_proposed,
                result.auto_accepted,
                result.pages_synced,
                result.backfill_complete,
                result.cursor.as_deref().unwrap_or("none"),
                connector.credential.is_some()
            );
        }
        Command::Connector {
            intent_id,
            adapter,
            source,
            token_env,
            refresh_token_env,
            token_url,
            client_id,
            client_secret_env,
            expires_at,
            credential_scopes,
            sync_interval,
            name,
        } => {
            let intent_id = IntentId::from(intent_id);
            let credential = build_connector_credential(
                &runtime,
                &adapter,
                token_env,
                refresh_token_env,
                token_url,
                client_id,
                client_secret_env,
                expires_at,
                credential_scopes,
            )?;
            let connector = runtime.register_connector_adapter_with_credential(
                &intent_id,
                &name,
                &source,
                &adapter,
                credential,
                sync_interval,
            )?;
            let result = runtime.sync_connector(&intent_id, &connector.id)?;
            println!(
                "connector: {} adapter={} observed={} inserted={} skipped={} candidates={} auto_accepted={} pages={} backfill_complete={} cursor={} credential={}",
                connector.id,
                connector.adapter,
                result.observed,
                result.inserted,
                result.skipped,
                result.candidates_proposed,
                result.auto_accepted,
                result.pages_synced,
                result.backfill_complete,
                result.cursor.as_deref().unwrap_or("none"),
                connector.credential.is_some()
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
            let intent_id = IntentId::from(intent_id);
            let projection = runtime.projection(&intent_id)?;
            for connector in projection.connectors.values() {
                let health = runtime.connector_health(&intent_id, &connector.id)?;
                println!(
                    "{} {:?} enabled={} health={:?} failures={} last_success={} next_sync={} pages={} backfill={} credential={} scopes={:?} provider={} sync={:?} read={:?} write={:?} source={}",
                    connector.id,
                    connector.adapter,
                    connector.enabled,
                    health.health,
                    health.consecutive_failures,
                    health
                        .last_success_at
                        .map(|value| value.to_rfc3339())
                        .unwrap_or_else(|| "never".into()),
                    health.next_sync_at.to_rfc3339(),
                    health.pages_synced,
                    if health.backfill_in_progress {
                        "in_progress"
                    } else if health.backfill_completed_at.is_some() {
                        "complete"
                    } else {
                        "none"
                    },
                    health.credential_configured,
                    health.credential_scopes,
                    connector.manifest.provider,
                    connector.manifest.sync_modes,
                    connector.manifest.read_capabilities,
                    connector.manifest.write_capabilities,
                    connector.source
                );
                if let Some(error) = health.last_error {
                    println!("  last_error={error}");
                }
            }
        }
        Command::ConnectorActionPropose {
            intent_id,
            connector_id,
            capability,
            target,
            input,
            idempotency_key,
        } => {
            let input = serde_json::from_str(&input).map_err(|error| {
                else_api::ApiError::Conflict(format!("action --input must be JSON: {error}"))
            })?;
            let action = runtime.propose_connector_action(
                &IntentId::from(intent_id),
                &else_protocol::ids::ConnectorId::from(connector_id),
                &capability,
                &target,
                input,
                idempotency_key,
            )?;
            print_connector_action(&action);
        }
        Command::ConnectorActionExecute {
            intent_id,
            action_id,
        } => {
            let action = runtime.execute_connector_action(
                &IntentId::from(intent_id),
                &else_protocol::ids::ConnectorActionId::from(action_id),
            )?;
            print_connector_action(&action);
        }
        Command::ConnectorActionVerify {
            intent_id,
            action_id,
        } => {
            let action = runtime.verify_connector_action(
                &IntentId::from(intent_id),
                &else_protocol::ids::ConnectorActionId::from(action_id),
            )?;
            print_connector_action(&action);
        }
        Command::ConnectorActionCompensate {
            intent_id,
            action_id,
        } => {
            let action = runtime.compensate_connector_action(
                &IntentId::from(intent_id),
                &else_protocol::ids::ConnectorActionId::from(action_id),
            )?;
            print_connector_action(&action);
        }
        Command::ConnectorActions { intent_id } => {
            for action in runtime.connector_actions(&IntentId::from(intent_id))? {
                print_connector_action(&action);
            }
        }
        Command::ConnectorCredentialRevoke {
            intent_id,
            connector_id,
        } => {
            let credential = runtime.revoke_connector_credential(
                &IntentId::from(intent_id),
                &else_protocol::ids::ConnectorId::from(connector_id),
            )?;
            println!(
                "credential revoked: {} auth={:?} revoked_at={}",
                credential.reference,
                credential.auth_mode,
                credential
                    .revoked_at
                    .map(|value| value.to_rfc3339())
                    .unwrap_or_default()
            );
        }
        Command::ConnectorCredentialSet {
            intent_id,
            connector_id,
            token_env,
            token_keychain_service,
        } => {
            let intent_id = IntentId::from(intent_id);
            let connector_id = else_protocol::ids::ConnectorId::from(connector_id);
            let connector = runtime
                .projection(&intent_id)?
                .connectors
                .get(&connector_id)
                .cloned()
                .ok_or_else(|| else_api::ApiError::NotFound(format!("connector {connector_id}")))?;
            let access_token = if let Some(service) = token_keychain_service {
                read_keychain_secret(&service)?
            } else {
                read_environment_secret(token_env.as_deref().unwrap_or("NOTION_TOKEN"))?
            };
            let credential =
                connector_credential_from_access_token(&runtime, &connector.adapter, access_token)?;
            let metadata =
                runtime.attach_connector_credential(&intent_id, &connector_id, credential)?;
            println!(
                "credential updated: {} auth={:?} scopes={:?} expires_at={}",
                metadata.reference,
                metadata.auth_mode,
                metadata.scopes,
                metadata
                    .expires_at
                    .map(|value| value.to_rfc3339())
                    .unwrap_or_else(|| "none".into())
            );
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
        Command::World {
            intent_id,
            include_resolved,
        } => {
            for item in runtime.world_items(&IntentId::from(intent_id), include_resolved)? {
                print_world_item(&item);
            }
        }
        Command::WorldResolve {
            intent_id,
            item_id,
            state,
            note,
        } => {
            let kind = match state.as_str() {
                "dismissed" => else_protocol::world::WorldItemResolutionKind::Dismissed,
                _ => else_protocol::world::WorldItemResolutionKind::Completed,
            };
            let item = runtime.resolve_world_item(
                &IntentId::from(intent_id),
                &else_protocol::ids::WorldItemId::from(item_id),
                kind,
                note,
            )?;
            print_world_item(&item);
        }
        Command::Brief => {
            let brief = runtime.personal_brief()?;
            println!("generated_at: {}", brief.generated_at.to_rfc3339());
            print_world_section("overdue", &brief.overdue);
            print_world_section("commitments", &brief.open_commitments);
            print_world_section("waiting_external", &brief.waiting_external);
            print_world_section("action_items_due_soon", &brief.action_items_due_soon);
            print_world_section("upcoming_calendar", &brief.upcoming_calendar);
            print_world_section("risks", &brief.risks);
        }
        Command::Projects {
            intent_id,
            include_resolved,
        } => {
            let intent_id = intent_id.map(IntentId::from);
            for project in runtime.project_worlds(intent_id.as_ref(), include_resolved)? {
                println!(
                    "{} {} open={} waiting={} overdue={} next_due={} intents={} connectors={}",
                    project.key,
                    project.title,
                    project.open_items,
                    project.waiting_external,
                    project.overdue,
                    project
                        .next_deadline
                        .map(|value| value.to_rfc3339())
                        .unwrap_or_else(|| "none".into()),
                    project.intent_ids.len(),
                    project.connector_ids.len()
                );
                for record in &project.items {
                    println!(
                        "  intent={} {:?} {:?} {}",
                        record.intent_id, record.item.kind, record.item.state, record.item.title
                    );
                }
            }
        }
        Command::Identities {
            intent_id,
            include_resolved,
        } => {
            let intent_id = intent_id.map(IntentId::from);
            for identity in runtime.identity_worlds(intent_id.as_ref(), include_resolved)? {
                println!(
                    "{:?} {} {} open={} overdue={} next_due={} intents={} connectors={}",
                    identity.kind,
                    identity.key,
                    identity.title,
                    identity.open_items,
                    identity.overdue,
                    identity
                        .next_deadline
                        .map(|value| value.to_rfc3339())
                        .unwrap_or_else(|| "none".into()),
                    identity.intent_ids.len(),
                    identity.connector_ids.len()
                );
                for record in &identity.items {
                    println!(
                        "  intent={} {:?} {:?} {}",
                        record.intent_id, record.item.kind, record.item.state, record.item.title
                    );
                }
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

fn print_world_section(name: &str, records: &[else_protocol::world::WorldItemRecord]) {
    println!("{name}: {}", records.len());
    for record in records {
        println!(
            "  intent={} {} {:?} {:?} due={} {}",
            record.intent_id,
            record.item.id,
            record.item.kind,
            record.item.state,
            record
                .item
                .deadline
                .map(|value| value.to_rfc3339())
                .unwrap_or_else(|| "none".into()),
            record.item.title
        );
    }
}

fn print_world_item(item: &else_protocol::world::WorldItem) {
    println!(
        "{} {:?} {:?} due={} signals={} connectors={} {}",
        item.id,
        item.kind,
        item.state,
        item.deadline
            .map(|value| value.to_rfc3339())
            .unwrap_or_else(|| "none".into()),
        item.signal_ids.len(),
        item.connector_ids.len(),
        item.title
    );
}

fn print_connector_action(action: &else_protocol::connector::ConnectorAction) {
    println!(
        "{} connector={} capability={} target={} status={:?} effect={:?} attention={} outcome={}",
        action.id,
        action.connector_id,
        action.capability,
        action.target,
        action.status,
        action.effect,
        action
            .attention_id
            .as_ref()
            .map(ToString::to_string)
            .unwrap_or_else(|| "none".into()),
        action
            .outcome
            .as_ref()
            .map(|outcome| format!("{:?}/{:?}", outcome.status, outcome.effect))
            .unwrap_or_else(|| "none".into())
    );
    if let Some(error) = &action.last_error {
        println!("  error={error}");
    }
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

fn notion_source_url(database: Option<&str>) -> else_api::Result<String> {
    let Some(database) = database.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok("https://api.notion.com/v1/search".into());
    };
    let without_query = database
        .split(['?', '#'])
        .next()
        .unwrap_or(database)
        .trim_end_matches('/');
    let path_segment = without_query.rsplit('/').next().unwrap_or(without_query);
    let compact = if path_segment.len() == 36
        && path_segment
            .chars()
            .all(|character| character.is_ascii_hexdigit() || character == '-')
    {
        path_segment.replace('-', "")
    } else if path_segment.len() == 32
        && path_segment
            .chars()
            .all(|character| character.is_ascii_hexdigit())
    {
        path_segment.to_string()
    } else {
        path_segment
            .rsplit('-')
            .next()
            .filter(|candidate| {
                candidate.len() == 32
                    && candidate
                        .chars()
                        .all(|character| character.is_ascii_hexdigit())
            })
            .map(str::to_string)
            .ok_or_else(|| {
                else_api::ApiError::Conflict(format!(
                    "could not find a Notion Database ID in {database}"
                ))
            })?
    };
    Ok(format!(
        "https://api.notion.com/v1/databases/{compact}/query"
    ))
}

fn google_calendar_source_url(calendar_id: &str) -> else_api::Result<String> {
    let calendar_id = calendar_id.trim();
    if calendar_id.is_empty() {
        return Err(else_api::ApiError::Conflict(
            "Google Calendar ID cannot be empty".into(),
        ));
    }
    Ok(format!(
        "https://www.googleapis.com/calendar/v3/calendars/{}/events",
        form_encode(calendar_id)
    ))
}

fn read_environment_secret(name: &str) -> else_api::Result<String> {
    let value = std::env::var(name).map_err(|_| {
        else_api::ApiError::Conflict(format!("environment variable {name} is not set"))
    })?;
    let value = value.trim().to_string();
    if value.is_empty() {
        return Err(else_api::ApiError::Conflict(format!(
            "environment variable {name} is empty"
        )));
    }
    Ok(value)
}

fn read_keychain_secret(service: &str) -> else_api::Result<String> {
    let output = std::process::Command::new("security")
        .args(["find-generic-password", "-s", service, "-w"])
        .output()
        .map_err(|error| {
            else_api::ApiError::Conflict(format!("cannot invoke macOS security: {error}"))
        })?;
    if !output.status.success() {
        return Err(else_api::ApiError::Unauthorized(format!(
            "Keychain service {service} was not found or access was denied"
        )));
    }
    let value = String::from_utf8(output.stdout)
        .map_err(|_| else_api::ApiError::Conflict("Keychain token is not UTF-8".into()))?
        .trim()
        .to_string();
    if value.is_empty() {
        return Err(else_api::ApiError::Conflict(format!(
            "Keychain service {service} is empty"
        )));
    }
    Ok(value)
}

fn connector_credential_from_access_token(
    runtime: &Runtime,
    adapter_id: &str,
    access_token: String,
) -> else_api::Result<else_protocol::connector::ConnectorCredential> {
    use else_protocol::connector::{ConnectorAuthMode, ConnectorCredential};

    let manifest = runtime
        .connector_manifests()
        .into_iter()
        .find(|manifest| manifest.adapter == adapter_id)
        .ok_or_else(|| else_api::ApiError::NotFound(format!("connector adapter {adapter_id}")))?;
    if manifest.auth_mode == ConnectorAuthMode::None {
        return Err(else_api::ApiError::Conflict(format!(
            "connector adapter {adapter_id} does not accept credentials"
        )));
    }
    Ok(ConnectorCredential {
        access_token,
        refresh_token: None,
        token_type: "bearer".into(),
        scopes: manifest.scopes,
        expires_at: None,
        refresh: None,
    })
}

fn form_encode(value: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            encoded.push(byte as char);
        } else {
            encoded.push('%');
            encoded.push(HEX[(byte >> 4) as usize] as char);
            encoded.push(HEX[(byte & 0x0f) as usize] as char);
        }
    }
    encoded
}

#[allow(clippy::too_many_arguments)]
fn build_connector_credential(
    runtime: &Runtime,
    adapter_id: &str,
    token_env: Option<String>,
    refresh_token_env: Option<String>,
    token_url: Option<String>,
    client_id: Option<String>,
    client_secret_env: Option<String>,
    expires_at: Option<String>,
    credential_scopes: Vec<String>,
) -> else_api::Result<Option<else_protocol::connector::ConnectorCredential>> {
    use else_protocol::connector::{ConnectorAuthMode, ConnectorCredential, OAuthRefreshConfig};

    let manifest = runtime
        .connector_manifests()
        .into_iter()
        .find(|manifest| manifest.adapter == adapter_id)
        .ok_or_else(|| else_api::ApiError::NotFound(format!("connector adapter {adapter_id}")))?;
    let Some(token_env) = token_env else {
        if refresh_token_env.is_some()
            || token_url.is_some()
            || client_id.is_some()
            || client_secret_env.is_some()
            || expires_at.is_some()
            || !credential_scopes.is_empty()
        {
            return Err(else_api::ApiError::Conflict(
                "--token-env is required when credential options are supplied".into(),
            ));
        }
        return Ok(None);
    };
    if manifest.auth_mode == ConnectorAuthMode::None {
        return Err(else_api::ApiError::Conflict(format!(
            "connector adapter {adapter_id} does not accept credentials"
        )));
    }
    let access_token = read_environment_secret(&token_env)?;
    let scopes = if credential_scopes.is_empty() {
        manifest.scopes
    } else {
        credential_scopes
    };
    let expires_at = expires_at
        .map(|value| {
            chrono::DateTime::parse_from_rfc3339(&value)
                .map(|value| value.with_timezone(&chrono::Utc))
                .map_err(|error| {
                    else_api::ApiError::Conflict(format!("--expires-at must be RFC3339: {error}"))
                })
        })
        .transpose()?;
    let refresh_token = refresh_token_env
        .as_deref()
        .map(|name| {
            std::env::var(name).map_err(|_| {
                else_api::ApiError::Conflict(format!("environment variable {name} is not set"))
            })
        })
        .transpose()?;
    let refresh = match (token_url, client_id) {
        (Some(token_url), Some(client_id)) => Some(OAuthRefreshConfig {
            token_url,
            client_id,
            client_secret: client_secret_env
                .as_deref()
                .map(|name| {
                    std::env::var(name).map_err(|_| {
                        else_api::ApiError::Conflict(format!(
                            "environment variable {name} is not set"
                        ))
                    })
                })
                .transpose()?,
        }),
        (None, None) => None,
        _ => {
            return Err(else_api::ApiError::Conflict(
                "--token-url and --client-id must be supplied together".into(),
            ))
        }
    };
    if refresh_token.is_some() && refresh.is_none() {
        return Err(else_api::ApiError::Conflict(
            "OAuth refresh requires --token-url and --client-id".into(),
        ));
    }
    Ok(Some(ConnectorCredential {
        access_token,
        refresh_token,
        token_type: "bearer".into(),
        scopes,
        expires_at,
        refresh,
    }))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notion_without_database_uses_search_endpoint() {
        assert_eq!(
            notion_source_url(None).unwrap(),
            "https://api.notion.com/v1/search"
        );
    }

    #[test]
    fn notion_database_url_is_normalized_to_query_endpoint() {
        let id = "0123456789abcdef0123456789abcdef";
        assert_eq!(
            notion_source_url(Some(&format!(
                "https://www.notion.so/workspace/Projects-{id}?v=123"
            )))
            .unwrap(),
            format!("https://api.notion.com/v1/databases/{id}/query")
        );
        assert_eq!(
            notion_source_url(Some(id)).unwrap(),
            format!("https://api.notion.com/v1/databases/{id}/query")
        );
    }

    #[test]
    fn google_calendar_source_encodes_calendar_identity() {
        assert_eq!(
            google_calendar_source_url("primary").unwrap(),
            "https://www.googleapis.com/calendar/v3/calendars/primary/events"
        );
        assert_eq!(
            google_calendar_source_url("team@example.com").unwrap(),
            "https://www.googleapis.com/calendar/v3/calendars/team%40example.com/events"
        );
    }
}
