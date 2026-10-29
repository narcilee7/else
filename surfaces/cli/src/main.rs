//! else CLI：Runtime API 的第一个薄客户端。
//! 只做参数解析与打印，不含业务逻辑。

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
    Intent { outcome: String },
    /// 对已有 Intent 再启动一次执行
    Run { intent_id: String },
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
        #[arg(long, default_value = "read_only", value_parser = ["read_only", "reversible", "compensatable", "irreversible", "unknown"])]
        max_effect: String,
    },
    /// 启动或续跑指定 Scope
    RunScope { intent_id: String, scope_id: String },
    /// 查看等待本人的 Decision
    Decisions { intent_id: String },
    /// 回答一个 Decision（选项 id 或自由文本）
    Decide {
        intent_id: String,
        request_id: String,
        choice: String,
    },
    /// 查看事件流（调试视图）
    Log { intent_id: String },
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
        Command::Intent { outcome } => {
            let spec = runtime.create_intent(&outcome)?;
            let intent_id = spec.id.clone();
            println!("intent: {intent_id}");
            let projection = runtime.run(&intent_id)?;
            print_projection(&projection);
        }
        Command::Run { intent_id } => {
            let projection = runtime.run(&IntentId::from(intent_id))?;
            print_projection(&projection);
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
            let child = runtime.spawn_subagent(
                &IntentId::from(intent_id),
                &else_protocol::ids::ScopeId::from(parent_scope),
                &task,
                max_effect,
            )?;
            println!("child scope: {child}");
        }
        Command::RunScope {
            intent_id,
            scope_id,
        } => {
            let projection = runtime.run_scope(
                &IntentId::from(intent_id),
                &else_protocol::ids::ScopeId::from(scope_id),
            )?;
            print_projection(&projection);
        }
        Command::Decisions { intent_id } => {
            let projection = runtime.projection(&IntentId::from(intent_id))?;
            print_decisions(&projection);
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
    }
    Ok(())
}

fn print_projection(projection: &else_ledger::IntentProjection) {
    println!("state: {:?}", projection.state);
    println!("artifacts: {}", projection.artifacts.len());
    print_decisions(projection);
}

fn print_decisions(projection: &else_ledger::IntentProjection) {
    if projection.open_requests.is_empty() {
        println!("decisions: none");
        return;
    }
    for (id, request) in &projection.open_requests {
        println!("decision {id}: {}", request.summary);
        for option in &request.options {
            println!("  [{}] {}", option.id, option.label);
        }
    }
}
