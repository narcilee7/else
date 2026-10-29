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
    /// 列出全部 Intent
    List,
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
        Command::List => {
            for spec in runtime.list_intents()? {
                println!("{}  {}", spec.id, spec.outcome);
            }
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
