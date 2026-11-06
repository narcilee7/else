//! else CLI：Runtime API 的第一个薄客户端。
//! 只做参数解析与打印，不含业务逻辑。
//!
//! CLI 与 macOS App 走**同一条 HTTP 链路**：同一个 daemon、同一份契约。
//! 因此终端里看到的 Brief，就是 App 里看到的那一份。

mod doctor;
mod error;
mod output;
mod service;
mod session;
mod tui;

use std::io::IsTerminal;
use std::path::PathBuf;

use clap::{Parser, Subcommand};
use else_client::RuntimeClient;
use else_protocol::attention::DecisionChoice;
use else_protocol::ids::{AttentionId, ThreadId};

use error::{CliError, Result};
use output::{
    print_attention, print_candidate, print_connector_action, print_decisions, print_plan,
    print_snapshot, print_world_item, print_world_section,
};
use session::Session;

#[derive(Parser)]
#[command(
    name = "else-bot",
    about = "You do what matters. Else handles the rest.",
    version = env!("CARGO_PKG_VERSION")
)]
struct Cli {
    /// Runtime 数据目录（事件库 + 工作区 + daemon-token）。
    ///
    /// 缺省用 `ELSE_RUNTIME_DIR`，再缺省用平台默认目录 —— macOS 上与
    /// App 后台 daemon 同址，所以终端和 App 看到的是同一个世界。
    #[arg(long)]
    dir: Option<PathBuf>,
    /// 以 JSON 输出结果，供脚本消费。
    #[arg(long, global = true)]
    json: bool,
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand, Clone)]
enum Command {
    /// 开一个 Thread：说清楚要达成什么，它自己开始做
    #[command(alias = "intent")]
    Thread {
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
    /// 推进一步：按当前状态继续做
    Advance {
        thread_id: String,
        /// 执行器偏好；auto 仍由 Kernel 按规则选择
        #[arg(long, default_value = "auto")]
        harness: String,
    },
    /// 一直做，直到需要你或没有可做的事
    Drive {
        thread_id: String,
        #[arg(long, default_value = "auto")]
        harness: String,
        #[arg(long, default_value_t = 8)]
        max_steps: u64,
    },
    /// 记录一个外部事件并唤醒该 Thread
    Event {
        thread_id: String,
        source: String,
        summary: String,
    },
    /// 标记 Thread 正在等待外部事实，停止空转执行
    WaitExternal {
        thread_id: String,
        #[arg(long, default_value = "waiting for external event")]
        reason: String,
    },
    /// 安排一个持久化 Kernel 唤醒 Timer
    Wake {
        thread_id: String,
        /// 从现在起多少秒触发。
        seconds: i64,
        #[arg(long, default_value = "follow up")]
        reason: String,
    },
    /// 对已有 Thread 再启动一次执行
    Run {
        thread_id: String,
        /// 执行器：scripted 或已注册的真实 Harness
        #[arg(long, default_value = "scripted")]
        harness: String,
    },
    /// 列出可用的 Harness 适配器及其能力声明
    Harnesses,
    /// 诊断：一次说清这台机器上的 else 是什么状态
    Doctor,
    /// 管理后台常驻的 Runtime 服务
    Daemon {
        #[command(subcommand)]
        action: DaemonCommand,
    },
    /// 首次运行：准备 runtime 目录并注册后台服务
    Onboard,
    /// 为已有 Thread 显式启用 Opaque Harness Profile
    AllowOpaque {
        thread_id: String,
        #[arg(long)]
        network: bool,
        #[arg(long)]
        tool_gateway: bool,
        #[arg(long, default_value = "read_only", value_parser = ["read_only", "reversible", "compensatable", "irreversible", "unknown"])]
        max_effect: String,
    },
    /// 取消执行并收敛在途副作用（可安全重复调用）
    Abort {
        thread_id: String,
        reason: Option<String>,
    },
    /// 列出全部 Thread
    List,
    /// 查看 Scope 委托树与生命周期
    Scopes { thread_id: String },
    /// 委托一个子 Scope（权限只能低于或等于父级）
    Delegate {
        thread_id: String,
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
        thread_id: String,
        scope_id: String,
        /// 执行器：scripted（默认）/ auto / codex / claude-code
        #[arg(long, default_value = "scripted")]
        harness: String,
    },
    /// 查看等你拍板的事
    Decisions { thread_id: String },
    /// 查看 Attention Window、批量率与误打断指标
    Attention { thread_id: String },
    /// 查看结构化 Plan 版本、步骤与外部事实影响
    Plan { thread_id: String },
    /// 回答一个 Decision（选项 id 或自由文本）
    Decide {
        thread_id: String,
        request_id: String,
        choice: String,
    },
    /// 查看事件流（调试视图）
    Log { thread_id: String },
    /// 查看模型、工具与环境资源预算及实际消耗
    Resources { thread_id: String },
    /// 创建检查点（事件水位 + 工作区快照 + 在途副作用结论）
    Checkpoint { thread_id: String },
    /// 从检查点派生分支并重跑（--alternative 走替代路径）
    Rerun {
        thread_id: String,
        checkpoint_id: String,
        #[arg(long)]
        alternative: bool,
    },
    /// 按 Generation 比较各执行分支（Outcome/副作用/产物）
    Compare { thread_id: String },
    /// 修订 Thread 目标（revision +1）；运行中的子分支默认取消，
    /// 用 --unaffected-scope 声明不受影响的分支
    Revise {
        thread_id: String,
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
        thread_id: String,
        /// 本地 ICS 文件或 HTTPS URL
        source: String,
        #[arg(long, default_value = "calendar")]
        name: String,
    },
    /// 注册真实 Google Calendar API Connector，并支持事件更新/核验/补偿
    GoogleCalendar {
        thread_id: String,
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
        thread_id: String,
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
        thread_id: String,
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
        thread_id: String,
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
        thread_id: String,
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
    Connectors { thread_id: String },
    /// 提案一次受 Gateway 管控的外部写动作
    ConnectorActionPropose {
        thread_id: String,
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
        thread_id: String,
        action_id: String,
    },
    /// 回源核验 Executing / Unknown / Applied 动作
    ConnectorActionVerify {
        thread_id: String,
        action_id: String,
    },
    /// 通过 Gateway 与 Decision 补偿已应用动作
    ConnectorActionCompensate {
        thread_id: String,
        action_id: String,
    },
    /// 查看外部写动作状态与最终结论
    ConnectorActions { thread_id: String },
    /// 撤销 Connector 私有凭据；后续同步失败关闭
    ConnectorCredentialRevoke {
        thread_id: String,
        connector_id: String,
    },
    /// 更新 Connector 私有凭据，不重复注册 Connector
    ConnectorCredentialSet {
        thread_id: String,
        connector_id: String,
        #[arg(long, conflicts_with = "token_keychain_service")]
        token_env: Option<String>,
        #[arg(long, conflicts_with = "token_env")]
        token_keychain_service: Option<String>,
    },
    /// 列出 Runtime 已注册的 Connector Adapter 与能力
    ConnectorAdapters,
    /// 查看标准化 World Signal 及失效状态
    Signals { thread_id: String },
    /// 查看承诺、等待外部、行动项与风险聚合出的 Personal World
    World {
        thread_id: String,
        #[arg(long)]
        include_resolved: bool,
    },
    /// 将 Personal World 条目标记完成或忽略
    WorldResolve {
        thread_id: String,
        item_id: String,
        #[arg(value_parser = ["completed", "dismissed"])]
        state: String,
        #[arg(long)]
        note: Option<String>,
    },
    /// 今天需要你管的事（跨所有 Thread）
    Brief,
    /// 按规范化 Project Key 查看跨 Provider World
    Projects {
        thread_id: Option<String>,
        #[arg(long)]
        include_resolved: bool,
    },
    /// 按 Contact / Customer / Thread / Milestone 查看跨 Provider 关系
    Identities {
        thread_id: Option<String>,
        #[arg(long)]
        include_resolved: bool,
    },
    /// 人工纠正 Project / Identity：merge / split / rename
    CorrelateCorrect {
        thread_id: String,
        #[arg(value_parser = ["merge", "split", "rename"])]
        action: String,
        #[arg(value_parser = ["project", "contact", "customer", "thread", "milestone"])]
        kind: String,
        #[arg(long = "source")]
        sources: Vec<String>,
        #[arg(long)]
        target: Option<String>,
        #[arg(long)]
        label: Option<String>,
        #[arg(long)]
        item_id: Option<String>,
    },
    /// 跨 Thread 按事件水位读取 Signal
    SignalFeed {
        #[arg(long, default_value_t = 0)]
        cursor: u64,
        #[arg(long)]
        provider: Option<String>,
        #[arg(long)]
        entity: Option<String>,
    },
    /// 跨 Thread 按事件水位读取 Thread Candidate
    CandidateFeed {
        #[arg(long, default_value_t = 0)]
        cursor: u64,
    },
    /// 运行确定性 Resolver，根据 Signal 提出 Thread Candidate
    ResolveSignals { thread_id: String },
    /// 查看 Thread Candidate
    Candidates { thread_id: String },
    /// 手工提出 Thread Candidate；只提案，不自动创建 Thread
    CandidatePropose {
        thread_id: String,
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
    /// 接受 Candidate，创建带完整来源血缘的 Thread
    CandidateAccept {
        thread_id: String,
        candidate_id: String,
    },
    /// 拒绝 Candidate，不创建 Thread
    CandidateReject {
        thread_id: String,
        candidate_id: String,
    },
}

#[derive(Subcommand, Clone)]
enum DaemonCommand {
    /// 注册后台服务，让 Runtime 常驻
    Install,
    /// 注销后台服务
    Uninstall,
    /// 查看后台服务状态
    Status,
}

#[derive(Subcommand, Clone)]
enum MemoryCommand {
    /// 列出记忆（默认行为）
    List,
    /// 用户确认晋升候选到指定作用域（global / project:P / thread:ID）
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
    // Clap 在 Cli 构造前就可能失败；先识别显式输出模式，保证参数错误也可机器解析。
    let requested_json = std::env::args_os()
        .take_while(|argument| argument != "--")
        .any(|argument| argument == "--json");
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(error) => {
            let failed = error.use_stderr();
            if requested_json && failed {
                eprintln!(
                    "{}",
                    serde_json::json!({
                        "error": { "kind": "usage", "message": error.to_string() }
                    })
                );
            } else {
                let _ = error.print();
            }
            std::process::exit(if failed { 2 } else { 0 });
        }
    };
    let json = cli.json;
    if let Err(error) = dispatch(cli) {
        if json {
            eprintln!(
                "{}",
                serde_json::json!({
                    "error": { "kind": error.kind(), "message": error.to_string() }
                })
            );
        } else {
            eprintln!("error: {error}");
        }
        std::process::exit(1);
    }
}

fn dispatch(cli: Cli) -> Result<()> {
    // Tool Gateway 与 daemon 都不经过 Runtime 会话：前者是 Harness 侧的被控
    // 通道，后者自己就是 Runtime。
    let Some(command) = cli.command.as_ref() else {
        // 没有子命令就是交互模式。
        let session = Session::open(cli.dir.as_deref())?;
        // 管道、CI、重定向与显式 --json 都输出简报，绝不启动交互界面。
        if cli.json || !std::io::stdout().is_terminal() {
            let client = session.client();
            let brief = client.brief()?;
            emit_or(cli.json, &brief, |brief| {
                println!("generated_at: {}", brief.generated_at.to_rfc3339());
                print_world_section("overdue", &brief.overdue);
                print_world_section("commitments", &brief.open_commitments);
                print_world_section("waiting_external", &brief.waiting_external);
                print_world_section("action_items_due_soon", &brief.action_items_due_soon);
                print_world_section("upcoming_calendar", &brief.upcoming_calendar);
                print_world_section("risks", &brief.risks);
            })?;
            return Ok(());
        }
        return tui::launch(session);
    };
    if let Command::ToolCall {
        tool,
        input,
        idempotency_key,
    } = command
    {
        run_tool_call(tool, input, idempotency_key.as_deref())?;
        return Ok(());
    }
    if let Command::Serve { bind, token } = command {
        let daemon = else_daemon::Daemon::start(else_daemon::DaemonConfig {
            dir: session::resolve_runtime_dir(cli.dir.as_deref()),
            bind: bind.clone(),
            token: token.clone(),
        })
        .map_err(|error| CliError::usage(error.to_string()))?;
        let token_file = session::resolve_runtime_dir(cli.dir.as_deref()).join("daemon-token");
        let startup = serde_json::json!({ "runtime": &daemon.url, "token_file": token_file });
        emit_or(cli.json, &startup, |startup| {
            println!(
                "runtime: {}",
                startup["runtime"].as_str().unwrap_or_default()
            );
            println!(
                "token-file: {}",
                startup["token_file"].as_str().unwrap_or_default()
            );
        })?;
        loop {
            std::thread::park_timeout(std::time::Duration::from_secs(60));
        }
    }

    let session = Session::open(cli.dir.as_deref())?;
    let client = session.client();

    match command.clone() {
        Command::Thread {
            outcome,
            project,
            harness,
            allow_opaque,
            opaque_network,
            tool_gateway,
            opaque_max_effect,
        } => {
            let snapshot = client.create_thread(&outcome, project.as_deref())?;
            let thread_id = snapshot.spec.id.clone();
            if allow_opaque {
                client.configure_opaque_harness(
                    &thread_id,
                    else_protocol::thread::OpaqueHarnessPolicy {
                        enabled: true,
                        allow_native_provider_network: opaque_network,
                        use_tool_gateway: tool_gateway,
                        max_observed_effect: parse_effect(&opaque_max_effect),
                    },
                )?;
            }
            let advanced = client.advance(&thread_id, Some(&harness))?;
            emit_or(cli.json, &advanced, |snapshot| {
                println!("thread: {thread_id}");
                print_snapshot(snapshot);
            })?;
        }
        Command::Advance { thread_id, harness } => {
            let advanced = client.advance(&ThreadId::from(thread_id), Some(&harness))?;
            emit_or(cli.json, &advanced, print_snapshot)?;
        }
        Command::Drive {
            thread_id,
            harness,
            max_steps,
        } => {
            let driven = client.drive(&ThreadId::from(thread_id), Some(&harness), max_steps)?;
            emit_or(cli.json, &driven, |driven| {
                println!("stopped: {}", driven.stopped);
                println!("steps: {}", driven.steps);
                print_snapshot(&driven.snapshot);
            })?;
        }
        Command::Event {
            thread_id,
            source,
            summary,
        } => {
            let projection =
                client.record_external_event(&ThreadId::from(thread_id), &source, &summary)?;
            emit_or(cli.json, &projection, print_snapshot)?;
        }
        Command::WaitExternal { thread_id, reason } => {
            let projection = client.wait_for_external(&ThreadId::from(thread_id), &reason)?;
            emit_or(cli.json, &projection, print_snapshot)?;
        }
        Command::Wake {
            thread_id,
            seconds,
            reason,
        } => {
            let timer_id = client.schedule_timer(&ThreadId::from(thread_id), seconds, &reason)?;
            let output = serde_json::json!({ "timer_id": timer_id });
            emit_or(cli.json, &output, |output| {
                println!("timer: {}", output["timer_id"].as_str().unwrap_or_default());
            })?;
        }
        Command::Run { thread_id, harness } => {
            let thread_id = ThreadId::from(thread_id);
            let available = client.harnesses()?;
            if harness != else_harness::SCRIPTED
                && !available.iter().any(|item| item.name == harness)
            {
                let names = available
                    .iter()
                    .map(|item| item.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ");
                return Err(CliError::usage(format!(
                    "harness {harness}（可用：{}{names}）",
                    if names.is_empty() {
                        "scripted"
                    } else {
                        "scripted, "
                    }
                )));
            }
            let projection = client.run(&thread_id, Some(&harness))?;
            emit_or(cli.json, &projection, print_snapshot)?;
        }
        Command::Doctor => {
            let runtime_dir = session::resolve_runtime_dir(cli.dir.as_deref());
            let report = doctor::run(client, &runtime_dir);
            if cli.json {
                emit_json(&report)?;
            } else {
                report.print();
            }
        }
        Command::Daemon { action } => {
            let runtime_dir = session::resolve_runtime_dir(cli.dir.as_deref());
            let status = match action {
                DaemonCommand::Install => service::install(&runtime_dir)?,
                DaemonCommand::Uninstall => service::uninstall(&runtime_dir)?,
                DaemonCommand::Status => service::status(&runtime_dir),
            };
            if cli.json {
                emit_json(&status)?;
            } else {
                println!("platform: {}", status.platform);
                println!("installed: {}", status.installed);
                println!("loaded: {}", status.loaded);
                if let Some(path) = &status.unit_path {
                    println!("unit: {}", path.display());
                }
                println!("{}", status.detail);
            }
        }
        Command::Onboard => {
            let runtime_dir = session::resolve_runtime_dir(cli.dir.as_deref());
            std::fs::create_dir_all(&runtime_dir).map_err(|error| {
                CliError::usage(format!("cannot create the runtime directory: {error}"))
            })?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&runtime_dir, std::fs::Permissions::from_mode(0o700))
                    .map_err(|error| {
                        CliError::usage(format!("cannot secure the runtime directory: {error}"))
                    })?;
            }
            let status = service::install(&runtime_dir)?;
            let output = serde_json::json!({ "runtime_dir": runtime_dir, "service": status });
            emit_or(cli.json, &output, |output| {
                println!(
                    "runtime directory: {}",
                    output["runtime_dir"].as_str().unwrap_or_default()
                );
                println!(
                    "background service: {}",
                    output["service"]["detail"].as_str().unwrap_or_default()
                );
                println!();
                println!("next:");
                println!("  else-bot              打开终端界面");
                println!("  else-bot brief        看今天要你管的事");
                println!("  else-bot doctor       再检查一次");
            })?;
        }
        Command::Harnesses => {
            let harnesses = client.harnesses()?;
            if cli.json {
                emit_json(&harnesses)?;
                return Ok(());
            }
            println!("scripted（内置确定性执行器）");
            for harness in harnesses {
                let caps = harness.capabilities;
                println!(
                    "{}（cancel={:?} usage={:?} tool_effects={:?} network={:?}）",
                    harness.name,
                    caps.cancel,
                    caps.usage,
                    caps.tool_effects,
                    harness.network_access
                );
            }
        }
        Command::AllowOpaque {
            thread_id,
            network,
            tool_gateway,
            max_effect,
        } => {
            let spec = client.configure_opaque_harness(
                &ThreadId::from(thread_id),
                else_protocol::thread::OpaqueHarnessPolicy {
                    enabled: true,
                    allow_native_provider_network: network,
                    use_tool_gateway: tool_gateway,
                    max_observed_effect: parse_effect(&max_effect),
                },
            )?;
            emit_or(cli.json, &spec, |spec| {
                println!(
                    "opaque: enabled network={} max_effect={:?} revision={}",
                    spec.opaque_harness_policy.allow_native_provider_network,
                    spec.opaque_harness_policy.max_observed_effect,
                    spec.revision
                );
            })?;
        }
        Command::Abort { thread_id, reason } => {
            let projection = client.abort(
                &ThreadId::from(thread_id),
                reason.as_deref().unwrap_or("用户取消"),
            )?;
            emit_or(cli.json, &projection, print_snapshot)?;
        }
        Command::List => {
            let threads = client.list_threads()?;
            if cli.json {
                emit_json(&threads)?;
                return Ok(());
            }
            for snapshot in threads {
                println!("{}  {}", snapshot.spec.id, snapshot.spec.outcome);
            }
        }
        Command::Scopes { thread_id } => {
            let scopes = client.scopes(&ThreadId::from(thread_id))?;
            if cli.json {
                emit_json(&scopes)?;
                return Ok(());
            }
            for scope in scopes {
                println!(
                    "scope {} parent={:?} replaces={:?} state={:?} generation={} failures={}",
                    scope.scope_id,
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
            thread_id,
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
            let parent = else_protocol::ids::ScopeId::from(parent_scope);
            let child = client.delegate(
                &ThreadId::from(thread_id),
                Some(&parent),
                &task,
                &depends_on,
                max_effect,
            )?;
            let output = serde_json::json!({ "scope_id": child });
            emit_or(cli.json, &output, |output| {
                println!(
                    "child scope: {}",
                    output["scope_id"].as_str().unwrap_or_default()
                );
            })?;
        }
        Command::RunScope {
            thread_id,
            scope_id,
            harness,
        } => {
            let projection = client.run_scope(
                &ThreadId::from(thread_id),
                &else_protocol::ids::ScopeId::from(scope_id),
                Some(&harness),
            )?;
            emit_or(cli.json, &projection, print_snapshot)?;
        }
        Command::Decisions { thread_id } => {
            let thread_id = ThreadId::from(thread_id);
            let windows: Vec<_> = client
                .attention_windows()?
                .into_iter()
                .filter(|window| window.thread_id == thread_id)
                .collect();
            let requests = client.thread(&thread_id)?.decisions;
            let output = serde_json::json!({ "windows": &windows, "requests": &requests });
            emit_or(cli.json, &output, |_| print_decisions(&windows, &requests))?;
        }
        Command::Attention { thread_id } => {
            let view = client.attention(&ThreadId::from(thread_id))?;
            if cli.json {
                emit_json(&view)?;
                return Ok(());
            }
            print_attention(&view);
        }
        Command::Plan { thread_id } => {
            let plan = client.plan(&ThreadId::from(thread_id))?;
            if cli.json {
                emit_json(&plan)?;
                return Ok(());
            }
            print_plan(&plan);
        }
        Command::Decide {
            thread_id,
            request_id,
            choice,
        } => {
            let choice = match choice.as_str() {
                "approve" | "reject" => DecisionChoice::Option(choice),
                other => DecisionChoice::FreeText(other.to_string()),
            };
            let projection = client.decide(
                &ThreadId::from(thread_id),
                &AttentionId::from(request_id),
                choice,
            )?;
            emit_or(cli.json, &projection, print_snapshot)?;
        }
        Command::Log { thread_id } => {
            let events = client.events(&ThreadId::from(thread_id))?;
            if cli.json {
                emit_json(&events)?;
                return Ok(());
            }
            for event in events {
                println!(
                    "#{:<4} {}",
                    event.seq,
                    serde_json::to_string(&event.payload)
                        .unwrap_or_else(|_| "<codec error>".into())
                );
            }
        }
        Command::Resources { thread_id } => {
            let report = client.resources(&ThreadId::from(thread_id))?;
            if cli.json {
                emit_json(&report)?;
                return Ok(());
            }
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
        Command::Checkpoint { thread_id } => {
            let checkpoint = client.checkpoint(&ThreadId::from(thread_id))?;
            if cli.json {
                emit_json(&checkpoint)?;
                return Ok(());
            }
            println!("checkpoint: {}", checkpoint.id);
            println!("event_cursor: {}", checkpoint.event_cursor);
            println!("inflight_effects: {}", checkpoint.inflight.len());
        }
        Command::Rerun {
            thread_id,
            checkpoint_id,
            alternative,
        } => {
            let thread_id = ThreadId::from(thread_id);
            let (scope_id, generation) = client.rerun(&thread_id, &checkpoint_id, alternative)?;
            let snapshot = client.thread(&thread_id)?;
            let output = serde_json::json!({
                "scope_id": scope_id,
                "generation": generation,
                "snapshot": &snapshot,
            });
            emit_or(cli.json, &output, |output| {
                println!(
                    "branch scope: {}",
                    output["scope_id"].as_str().unwrap_or_default()
                );
                println!("generation: {generation}");
                print_snapshot(&snapshot);
            })?;
        }
        Command::Compare { thread_id } => {
            let comparison = client.compare(&ThreadId::from(thread_id))?;
            let output = serde_json::json!({ "comparison": comparison });
            emit_or(cli.json, &output, |output| {
                print!("{}", output["comparison"].as_str().unwrap_or_default());
            })?;
        }
        Command::Revise {
            thread_id,
            outcome,
            constraints,
            reason,
            unaffected_scopes: _,
        } => {
            let spec = client.revise_thread(
                &ThreadId::from(thread_id),
                outcome.as_deref(),
                &constraints,
                &reason,
            )?;
            emit_or(cli.json, &spec, |spec| {
                println!("revision: {}", spec.revision);
                println!("outcome: {}", spec.outcome);
            })?;
        }
        Command::Memory { command, project } => {
            use else_protocol::memory::{MemoryScope, MemoryTier};
            let parse_scope = |raw: &str| -> std::result::Result<MemoryScope, String> {
                if raw == "global" {
                    Ok(MemoryScope::Global)
                } else if let Some(name) = raw.strip_prefix("project:") {
                    Ok(MemoryScope::Project(name.to_string()))
                } else if let Some(id) = raw.strip_prefix("thread:") {
                    Ok(MemoryScope::Thread(ThreadId::from(id)))
                } else {
                    Err(format!(
                        "unknown scope {raw} (global / project:P / thread:ID)"
                    ))
                }
            };
            match command {
                None | Some(MemoryCommand::List) => {
                    let memory = client.memory()?;
                    let tiers = [
                        MemoryTier::Working,
                        MemoryTier::Episodic,
                        MemoryTier::Semantic,
                    ];
                    let recalled = else_protocol::memory::recall_memories(
                        memory.promoted.values(),
                        project.as_deref(),
                        None,
                        &tiers,
                        chrono::Utc::now(),
                    );
                    let candidates: Vec<_> = if project.is_none() {
                        memory.candidates.values().collect()
                    } else {
                        Vec::new()
                    };
                    let output = serde_json::json!({
                        "promoted": &recalled,
                        "candidates": &candidates,
                    });
                    emit_or(cli.json, &output, |_| {
                        println!("promoted (scope={:?}):", project.as_deref().unwrap_or("*"));
                        for record in &recalled {
                            println!(
                                "  {} {:?} {:?} {} = {}",
                                record.id, record.tier, record.scope, record.subject, record.value
                            );
                        }
                        if project.is_none() {
                            println!("candidates:");
                            for record in candidates {
                                println!(
                                    "  {} {:?} {:?} {} = {}",
                                    record.id,
                                    record.tier,
                                    record.scope,
                                    record.subject,
                                    record.value
                                );
                            }
                        }
                    })?;
                }
                Some(MemoryCommand::Promote {
                    candidate_id,
                    scope,
                }) => {
                    let scope = scope
                        .map(|raw| parse_scope(&raw))
                        .transpose()
                        .map_err(CliError::usage)?;
                    let record = client.promote_memory(
                        &else_protocol::ids::MemoryId::from(candidate_id),
                        scope.as_ref(),
                    )?;
                    emit_or(cli.json, &record, |record| {
                        println!("promoted: {} -> {:?}", record.id, record.scope);
                    })?;
                }
                Some(MemoryCommand::Revoke { memory_id, reason }) => {
                    client.revoke_memory(
                        &else_protocol::ids::MemoryId::from(memory_id.clone()),
                        &reason,
                    )?;
                    let output = serde_json::json!({ "memory_id": memory_id, "revoked": true });
                    emit_or(cli.json, &output, |output| {
                        println!(
                            "revoked: {}",
                            output["memory_id"].as_str().unwrap_or_default()
                        );
                    })?;
                }
                Some(MemoryCommand::Correct {
                    project,
                    subject,
                    predicate,
                    value,
                }) => {
                    let record = client.correct_memory(&project, &subject, &predicate, &value)?;
                    emit_or(cli.json, &record, |record| {
                        println!("corrected: {} (project scope only)", record.id);
                    })?;
                }
            }
        }
        Command::ToolCall { .. } => unreachable!("handled before the session opens"),
        Command::Serve { .. } => unreachable!("handled before the session opens"),
        Command::Calendar {
            thread_id,
            source,
            name,
        } => {
            let thread_id = ThreadId::from(thread_id);
            let connector =
                client.register_connector(&thread_id, &name, &source, "calendar.ics", None, 300)?;
            let result = client.sync_connector(&thread_id, &connector.id)?;
            if cli.json {
                emit_json(&serde_json::json!({ "connector": connector, "sync": result }))?;
                return Ok(());
            }
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
            thread_id,
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
            let thread_id = ThreadId::from(thread_id);
            let source = google_calendar_source_url(&calendar_id)?;
            let credential = if let Some(service) = token_keychain_service {
                Some(connector_credential_from_access_token(
                    client,
                    "calendar.google",
                    read_keychain_secret(&service)?,
                )?)
            } else {
                build_connector_credential(
                    client,
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
            let connector = client.register_connector(
                &thread_id,
                &name,
                &source,
                "calendar.google",
                credential.as_ref(),
                sync_interval,
            )?;
            let result = client.sync_connector(&thread_id, &connector.id)?;
            if cli.json {
                emit_json(&serde_json::json!({ "connector": connector, "sync": result }))?;
                return Ok(());
            }
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
            thread_id,
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
            let thread_id = ThreadId::from(thread_id);
            let credential = build_connector_credential(
                client,
                "linear.graphql",
                token_env,
                refresh_token_env,
                token_url,
                client_id,
                client_secret_env,
                expires_at,
                credential_scopes,
            )?;
            let connector = client.register_connector(
                &thread_id,
                &name,
                &source,
                "linear.graphql",
                credential.as_ref(),
                sync_interval,
            )?;
            let result = client.sync_connector(&thread_id, &connector.id)?;
            if cli.json {
                emit_json(&serde_json::json!({ "connector": connector, "sync": result }))?;
                return Ok(());
            }
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
            thread_id,
            database,
            token_env,
            token_keychain_service,
            sync_interval,
            name,
        } => {
            let thread_id = ThreadId::from(thread_id);
            let source = notion_source_url(database.as_deref())?;
            let access_token = if let Some(service) = token_keychain_service {
                read_keychain_secret(&service)?
            } else {
                read_environment_secret(token_env.as_deref().unwrap_or("NOTION_TOKEN"))?
            };
            let credential =
                connector_credential_from_access_token(client, "notion.api", access_token)?;
            let connector = client.register_connector(
                &thread_id,
                &name,
                &source,
                "notion.api",
                Some(&credential),
                sync_interval,
            )?;
            let result = client.sync_connector(&thread_id, &connector.id)?;
            if cli.json {
                emit_json(&serde_json::json!({ "connector": connector, "sync": result }))?;
                return Ok(());
            }
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
            thread_id,
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
            let thread_id = ThreadId::from(thread_id);
            let credential = build_connector_credential(
                client,
                &adapter,
                token_env,
                refresh_token_env,
                token_url,
                client_id,
                client_secret_env,
                expires_at,
                credential_scopes,
            )?;
            let connector = client.register_connector(
                &thread_id,
                &name,
                &source,
                &adapter,
                credential.as_ref(),
                sync_interval,
            )?;
            let result = client.sync_connector(&thread_id, &connector.id)?;
            if cli.json {
                emit_json(&serde_json::json!({ "connector": connector, "sync": result }))?;
                return Ok(());
            }
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
            thread_id,
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
                        .map(|origin| parse_thread_origin(origin))
                        .collect()
                },
                min_confidence,
                max_autonomous_effect: parse_effect(&max_effect),
            };
            let spec = client.configure_connector_policy(&ThreadId::from(thread_id), policy)?;
            emit_or(cli.json, &spec, |spec| {
                println!(
                    "connector policy: auto_accept={} min_confidence={} max_effect={:?} revision={}",
                    spec.connector_policy.auto_accept_low_risk,
                    spec.connector_policy.min_confidence,
                    spec.connector_policy.max_autonomous_effect,
                    spec.revision
                );
            })?;
        }
        Command::Connectors { thread_id } => {
            let thread_id = ThreadId::from(thread_id);
            let connectors = client.connectors(&thread_id)?;
            if cli.json {
                let mut rows = Vec::with_capacity(connectors.len());
                for connector in connectors {
                    let health = client.connector_health(&thread_id, &connector.id)?;
                    rows.push(serde_json::json!({ "connector": connector, "health": health }));
                }
                emit_json(&rows)?;
                return Ok(());
            }
            for connector in connectors {
                let health = client.connector_health(&thread_id, &connector.id)?;
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
            thread_id,
            connector_id,
            capability,
            target,
            input,
            idempotency_key,
        } => {
            let input = serde_json::from_str(&input).map_err(|error| {
                CliError::usage(format!("action --input must be JSON: {error}"))
            })?;
            let action = client.propose_connector_action(
                &ThreadId::from(thread_id),
                &else_protocol::ids::ConnectorId::from(connector_id),
                &capability,
                &target,
                input,
                idempotency_key.as_deref(),
            )?;
            emit_or(cli.json, &action, print_connector_action)?;
        }
        Command::ConnectorActionExecute {
            thread_id,
            action_id,
        } => {
            let action = client.connector_action(
                &ThreadId::from(thread_id),
                &else_protocol::ids::ConnectorActionId::from(action_id),
                "execute",
            )?;
            emit_or(cli.json, &action, print_connector_action)?;
        }
        Command::ConnectorActionVerify {
            thread_id,
            action_id,
        } => {
            let action = client.connector_action(
                &ThreadId::from(thread_id),
                &else_protocol::ids::ConnectorActionId::from(action_id),
                "verify",
            )?;
            emit_or(cli.json, &action, print_connector_action)?;
        }
        Command::ConnectorActionCompensate {
            thread_id,
            action_id,
        } => {
            let action = client.connector_action(
                &ThreadId::from(thread_id),
                &else_protocol::ids::ConnectorActionId::from(action_id),
                "compensate",
            )?;
            emit_or(cli.json, &action, print_connector_action)?;
        }
        Command::ConnectorActions { thread_id } => {
            let actions = client.connector_actions(&ThreadId::from(thread_id))?;
            emit_or(cli.json, &actions, |actions| {
                for action in actions {
                    print_connector_action(action);
                }
            })?;
        }
        Command::ConnectorCredentialRevoke {
            thread_id,
            connector_id,
        } => {
            let credential = client.revoke_connector_credential(
                &ThreadId::from(thread_id),
                &else_protocol::ids::ConnectorId::from(connector_id),
            )?;
            emit_or(cli.json, &credential, |credential| {
                println!(
                    "credential revoked: {} auth={:?} revoked_at={}",
                    credential.reference,
                    credential.auth_mode,
                    credential
                        .revoked_at
                        .map(|value| value.to_rfc3339())
                        .unwrap_or_default()
                );
            })?;
        }
        Command::ConnectorCredentialSet {
            thread_id,
            connector_id,
            token_env,
            token_keychain_service,
        } => {
            let thread_id = ThreadId::from(thread_id);
            let connector_id = else_protocol::ids::ConnectorId::from(connector_id);
            let connector = client
                .connectors(&thread_id)?
                .into_iter()
                .find(|connector| connector.id == connector_id)
                .ok_or_else(|| CliError::usage(format!("connector {connector_id}")))?;
            let access_token = if let Some(service) = token_keychain_service {
                read_keychain_secret(&service)?
            } else {
                read_environment_secret(token_env.as_deref().unwrap_or("NOTION_TOKEN"))?
            };
            let credential =
                connector_credential_from_access_token(client, &connector.adapter, access_token)?;
            let metadata =
                client.attach_connector_credential(&thread_id, &connector_id, credential)?;
            emit_or(cli.json, &metadata, |metadata| {
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
            })?;
        }
        Command::ConnectorAdapters => {
            let manifests = client.connector_manifests()?;
            if cli.json {
                emit_json(&manifests)?;
                return Ok(());
            }
            for manifest in manifests {
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
        Command::Signals { thread_id } => {
            let signals = client.world_signals_of(&ThreadId::from(thread_id))?;
            if cli.json {
                emit_json(&signals)?;
                return Ok(());
            }
            for signal in signals {
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
            thread_id,
            include_resolved,
        } => {
            let items = client.world_items(&ThreadId::from(thread_id), include_resolved)?;
            if cli.json {
                emit_json(&items)?;
                return Ok(());
            }
            for item in items {
                print_world_item(&item);
            }
        }
        Command::WorldResolve {
            thread_id,
            item_id,
            state,
            note,
        } => {
            let kind = match state.as_str() {
                "dismissed" => else_protocol::world::WorldItemResolutionKind::Dismissed,
                _ => else_protocol::world::WorldItemResolutionKind::Completed,
            };
            let item = client.resolve_world_item(
                &ThreadId::from(thread_id),
                &else_protocol::ids::WorldItemId::from(item_id),
                kind,
                note.as_deref(),
            )?;
            emit_or(cli.json, &item, print_world_item)?;
        }
        Command::Brief => {
            let brief = client.brief()?;
            if cli.json {
                emit_json(&brief)?;
                return Ok(());
            }
            println!("generated_at: {}", brief.generated_at.to_rfc3339());
            print_world_section("overdue", &brief.overdue);
            print_world_section("commitments", &brief.open_commitments);
            print_world_section("waiting_external", &brief.waiting_external);
            print_world_section("action_items_due_soon", &brief.action_items_due_soon);
            print_world_section("upcoming_calendar", &brief.upcoming_calendar);
            print_world_section("risks", &brief.risks);
        }
        Command::Projects {
            thread_id,
            include_resolved,
        } => {
            let thread_id = thread_id.map(ThreadId::from);
            let projects = client.project_worlds(thread_id.as_ref(), include_resolved)?;
            if cli.json {
                emit_json(&projects)?;
                return Ok(());
            }
            for project in projects {
                println!(
                    "{} {} open={} waiting={} overdue={} next_due={} threads={} connectors={}",
                    project.key,
                    project.title,
                    project.open_items,
                    project.waiting_external,
                    project.overdue,
                    project
                        .next_deadline
                        .map(|value| value.to_rfc3339())
                        .unwrap_or_else(|| "none".into()),
                    project.thread_ids.len(),
                    project.connector_ids.len()
                );
                for record in &project.items {
                    println!(
                        "  thread={} {:?} {:?} {}",
                        record.thread_id, record.item.kind, record.item.state, record.item.title
                    );
                }
            }
        }
        Command::Identities {
            thread_id,
            include_resolved,
        } => {
            let thread_id = thread_id.map(ThreadId::from);
            let identities = client.identity_worlds(thread_id.as_ref(), include_resolved)?;
            if cli.json {
                emit_json(&identities)?;
                return Ok(());
            }
            for identity in identities {
                println!(
                    "{:?} {} {} open={} overdue={} next_due={} threads={} connectors={}",
                    identity.kind,
                    identity.key,
                    identity.title,
                    identity.open_items,
                    identity.overdue,
                    identity
                        .next_deadline
                        .map(|value| value.to_rfc3339())
                        .unwrap_or_else(|| "none".into()),
                    identity.thread_ids.len(),
                    identity.connector_ids.len()
                );
                for record in &identity.items {
                    println!(
                        "  thread={} {:?} {:?} {}",
                        record.thread_id, record.item.kind, record.item.state, record.item.title
                    );
                }
            }
        }
        Command::CorrelateCorrect {
            thread_id,
            action,
            kind,
            sources,
            target,
            label,
            item_id,
        } => {
            use else_protocol::world::{
                WorldCorrelationCorrection, WorldCorrelationCorrectionKind,
            };
            let kind = match kind.as_str() {
                "project" => else_protocol::connector::CorrelationKind::Project,
                "contact" => else_protocol::connector::CorrelationKind::Contact,
                "customer" => else_protocol::connector::CorrelationKind::Customer,
                "thread" => else_protocol::connector::CorrelationKind::Thread,
                "milestone" => else_protocol::connector::CorrelationKind::Milestone,
                _ => unreachable!("clap validates correlation kind"),
            };
            let action = match action.as_str() {
                "merge" => WorldCorrelationCorrectionKind::Merge,
                "split" => WorldCorrelationCorrectionKind::Split,
                "rename" => WorldCorrelationCorrectionKind::Rename,
                _ => unreachable!("clap validates correction action"),
            };
            let correction = client.correct_world_correlation(
                &ThreadId::from(thread_id),
                WorldCorrelationCorrection {
                    id: else_protocol::ids::WorldCorrelationCorrectionId::new(),
                    kind: action,
                    correlation_kind: kind,
                    source_keys: sources,
                    target_key: target,
                    label,
                    item_id: item_id.map(else_protocol::ids::WorldItemId::from),
                    corrected_at: chrono::Utc::now(),
                },
            )?;
            emit_or(cli.json, &correction, |correction| {
                println!(
                    "correction: {} {:?} {:?} target={:?} label={:?}",
                    correction.id,
                    correction.kind,
                    correction.correlation_kind,
                    correction.target_key,
                    correction.label
                );
            })?;
        }
        Command::SignalFeed {
            cursor,
            provider,
            entity,
        } => {
            let records = client.world_signals(cursor, provider.as_deref(), entity.as_deref())?;
            if cli.json {
                emit_json(&records)?;
                return Ok(());
            }
            for record in records {
                println!(
                    "#{} thread={} {} {:?} {:?} {:?} {}",
                    record.seq,
                    record.thread_id,
                    record.signal.id,
                    record.signal.object_kind,
                    record.signal.change,
                    record.signal.status,
                    record.signal.summary
                );
            }
        }
        Command::CandidateFeed { cursor } => {
            let records = client.thread_candidates(cursor)?;
            if cli.json {
                emit_json(&records)?;
                return Ok(());
            }
            for record in records {
                println!(
                    "#{} thread={} {} {:?} {:?} {}",
                    record.seq,
                    record.thread_id,
                    record.candidate.id,
                    record.candidate.origin,
                    record.candidate.state,
                    record.candidate.proposed_outcome
                );
            }
        }
        Command::ResolveSignals { thread_id } => {
            let candidates = client.resolve_signals(&ThreadId::from(thread_id))?;
            if cli.json {
                emit_json(&candidates)?;
                return Ok(());
            }
            if candidates.is_empty() {
                println!("candidates: none");
            }
            for candidate in candidates {
                print_candidate(&candidate);
            }
        }
        Command::Candidates { thread_id } => {
            let records = client.thread_candidates_of(&ThreadId::from(thread_id))?;
            if cli.json {
                emit_json(&records)?;
                return Ok(());
            }
            if records.is_empty() {
                println!("candidates: none");
            }
            for record in records {
                print_candidate(&record.candidate);
            }
        }
        Command::CandidatePropose {
            thread_id,
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
            use else_protocol::connector::{ThreadCandidate, ThreadCandidateState};
            let thread_id = ThreadId::from(thread_id);
            let origin = parse_thread_origin(&origin);
            let related_signal_ids = signals
                .into_iter()
                .map(else_protocol::ids::WorldSignalId::from)
                .collect::<Vec<_>>();
            let done_when = if done_when.is_empty() {
                vec![else_protocol::thread::Criterion::manual(format!(
                    "确认完成：{outcome}"
                ))]
            } else {
                done_when
                    .into_iter()
                    .map(else_protocol::thread::Criterion::manual)
                    .collect()
            };
            let deadline = deadline
                .map(|value| {
                    chrono::DateTime::parse_from_rfc3339(&value)
                        .map(|value| value.with_timezone(&chrono::Utc))
                        .map_err(|error| {
                            CliError::usage(format!("invalid deadline RFC3339: {error}"))
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
            let candidate = client.propose_thread_candidate(
                &thread_id,
                ThreadCandidate {
                    id: else_protocol::ids::ThreadCandidateId::new(),
                    origin,
                    proposed_outcome: outcome,
                    done_when,
                    constraints,
                    authority: else_protocol::thread::Authority {
                        max_autonomous_effect: parse_effect(&max_effect),
                        forbidden: vec![],
                    },
                    attention_policy: else_protocol::thread::AttentionPolicy {
                        allow_interrupt: false,
                        batch_interval_minutes: 60,
                    },
                    deadline,
                    related_signal_ids,
                    related_thread_id: Some(thread_id.clone()),
                    confidence,
                    dedupe_key,
                    state: ThreadCandidateState::Proposed,
                    accepted_thread_id: None,
                    accepted_by: None,
                },
            )?;
            emit_or(cli.json, &candidate, print_candidate)?;
        }
        Command::CandidateAccept {
            thread_id,
            candidate_id,
        } => {
            let (_, thread) = client.resolve_thread_candidate(
                &ThreadId::from(thread_id),
                &else_protocol::ids::ThreadCandidateId::from(candidate_id),
                true,
            )?;
            let spec =
                thread.ok_or_else(|| CliError::usage("candidate acceptance produced no thread"))?;
            emit_or(cli.json, &spec, |spec| {
                println!("thread: {}", spec.id);
                println!("outcome: {}", spec.outcome);
                if let Some(origin) = &spec.origin {
                    println!(
                        "origin: {:?} candidate={:?} signals={} connectors={}",
                        origin.kind,
                        origin.candidate_id,
                        origin.signal_ids.len(),
                        origin.connector_ids.len()
                    );
                }
            })?;
        }
        Command::CandidateReject {
            thread_id,
            candidate_id,
        } => {
            let (record, _) = client.resolve_thread_candidate(
                &ThreadId::from(thread_id),
                &else_protocol::ids::ThreadCandidateId::from(candidate_id),
                false,
            )?;
            emit_or(cli.json, &record, |_| println!("rejected"))?;
        }
    }
    Ok(())
}

/// 以 JSON 输出一个值。
fn emit_json<T: serde::Serialize>(value: &T) -> Result<()> {
    let text = serde_json::to_string_pretty(value)
        .map_err(|error| CliError::usage(format!("cannot encode JSON: {error}")))?;
    println!("{text}");
    Ok(())
}

/// 同一个领域结果只构造一次，按输出模式选择机器或人类呈现。
fn emit_or<T: serde::Serialize>(json: bool, value: &T, human: impl FnOnce(&T)) -> Result<()> {
    if json {
        emit_json(value)
    } else {
        human(value);
        Ok(())
    }
}

fn parse_thread_origin(origin: &str) -> else_protocol::connector::ThreadOriginKind {
    use else_protocol::connector::ThreadOriginKind;
    match origin {
        "user_direct" => ThreadOriginKind::UserDirect,
        "external_request" => ThreadOriginKind::ExternalRequest,
        "commitment" => ThreadOriginKind::Commitment,
        "agent_proposal" => ThreadOriginKind::AgentProposal,
        "system_recovery" => ThreadOriginKind::SystemRecovery,
        _ => ThreadOriginKind::InferredNeed,
    }
}

fn notion_source_url(database: Option<&str>) -> Result<String> {
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
                CliError::usage(format!("could not find a Notion Database ID in {database}"))
            })?
    };
    Ok(format!(
        "https://api.notion.com/v1/databases/{compact}/query"
    ))
}

fn google_calendar_source_url(calendar_id: &str) -> Result<String> {
    let calendar_id = calendar_id.trim();
    if calendar_id.is_empty() {
        return Err(CliError::usage("Google Calendar ID cannot be empty"));
    }
    Ok(format!(
        "https://www.googleapis.com/calendar/v3/calendars/{}/events",
        form_encode(calendar_id)
    ))
}

fn read_environment_secret(name: &str) -> Result<String> {
    let value = std::env::var(name)
        .map_err(|_| CliError::usage(format!("environment variable {name} is not set")))?;
    let value = value.trim().to_string();
    if value.is_empty() {
        return Err(CliError::usage(format!(
            "environment variable {name} is empty"
        )));
    }
    Ok(value)
}

fn read_keychain_secret(service: &str) -> Result<String> {
    let output = std::process::Command::new("security")
        .args(["find-generic-password", "-s", service, "-w"])
        .output()
        .map_err(|error| CliError::usage(format!("cannot invoke macOS security: {error}")))?;
    if !output.status.success() {
        return Err(CliError::usage(format!(
            "Keychain service {service} was not found or access was denied"
        )));
    }
    let value = String::from_utf8(output.stdout)
        .map_err(|_| CliError::usage("Keychain token is not UTF-8"))?
        .trim()
        .to_string();
    if value.is_empty() {
        return Err(CliError::usage(format!(
            "Keychain service {service} is empty"
        )));
    }
    Ok(value)
}

fn connector_credential_from_access_token(
    client: &RuntimeClient,
    adapter_id: &str,
    access_token: String,
) -> Result<else_protocol::connector::ConnectorCredential> {
    use else_protocol::connector::{ConnectorAuthMode, ConnectorCredential};

    let manifest = client
        .connector_manifests()?
        .into_iter()
        .find(|manifest| manifest.adapter == adapter_id)
        .ok_or_else(|| CliError::usage(format!("connector adapter {adapter_id}")))?;
    if manifest.auth_mode == ConnectorAuthMode::None {
        return Err(CliError::usage(format!(
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
    client: &RuntimeClient,
    adapter_id: &str,
    token_env: Option<String>,
    refresh_token_env: Option<String>,
    token_url: Option<String>,
    client_id: Option<String>,
    client_secret_env: Option<String>,
    expires_at: Option<String>,
    credential_scopes: Vec<String>,
) -> Result<Option<else_protocol::connector::ConnectorCredential>> {
    use else_protocol::connector::{ConnectorAuthMode, ConnectorCredential, OAuthRefreshConfig};

    let manifest = client
        .connector_manifests()?
        .into_iter()
        .find(|manifest| manifest.adapter == adapter_id)
        .ok_or_else(|| CliError::usage(format!("connector adapter {adapter_id}")))?;
    let Some(token_env) = token_env else {
        if refresh_token_env.is_some()
            || token_url.is_some()
            || client_id.is_some()
            || client_secret_env.is_some()
            || expires_at.is_some()
            || !credential_scopes.is_empty()
        {
            return Err(CliError::usage(
                "--token-env is required when credential options are supplied",
            ));
        }
        return Ok(None);
    };
    if manifest.auth_mode == ConnectorAuthMode::None {
        return Err(CliError::usage(format!(
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
                .map_err(|error| CliError::usage(format!("--expires-at must be RFC3339: {error}")))
        })
        .transpose()?;
    let refresh_token = refresh_token_env
        .as_deref()
        .map(|name| {
            std::env::var(name)
                .map_err(|_| CliError::usage(format!("environment variable {name} is not set")))
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
                        CliError::usage(format!("environment variable {name} is not set"))
                    })
                })
                .transpose()?,
        }),
        (None, None) => None,
        _ => {
            return Err(CliError::usage(
                "--token-url and --client-id must be supplied together",
            ))
        }
    };
    if refresh_token.is_some() && refresh.is_none() {
        return Err(CliError::usage(
            "OAuth refresh requires --token-url and --client-id",
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

fn run_tool_call(tool: &str, input: &str, idempotency_key: Option<&str>) -> Result<()> {
    use std::io::{Read, Write};
    use std::net::TcpStream;

    let url =
        std::env::var("ELSE_TOOL_URL").map_err(|_| CliError::usage("ELSE_TOOL_URL is not set"))?;
    let token = std::env::var("ELSE_TOOL_TOKEN")
        .map_err(|_| CliError::usage("ELSE_TOOL_TOKEN is not set"))?;
    let authority = url
        .strip_prefix("http://")
        .ok_or_else(|| CliError::usage("ELSE_TOOL_URL must be http loopback"))?;
    let input: serde_json::Value = serde_json::from_str(input)
        .map_err(|error| CliError::usage(format!("invalid tool input JSON: {error}")))?;
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
        .map_err(|error| CliError::usage(format!("tool gateway: {error}")))?;
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
        Err(CliError::usage(body.to_string()))
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
