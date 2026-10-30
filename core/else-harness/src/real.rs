//! 真实 Harness 适配器（MVP-2）：Codex 与 Claude Code。
//!
//! 两者都以非交互方式接入，事件从各自输出流归一化桥接：
//! - Codex：`codex exec --json`（JSONL，格式经本机 0.166.0 实测）；
//! - Claude Code：`claude -p --output-format stream-json --verbose`（2.1.284 实测）。
//!
//! 私有配置留在适配器：Codex 凭据暂存到 Runtime 控制的 `harness-private/`，
//! 通过 CODEX_HOME 注入；运行结束立即删除，且不进入 Workspace 快照。
//! 模型流量走 Harness 原生 Provider：网络按显式策略放通出站（Opaque），
//! 凭据在沙箱内且网络可达外部——这是已声明的 Opaque 边界，出口代理收敛后置。

use std::path::{Path, PathBuf};

use else_environment::NetworkPolicy;
use else_protocol::effect::SideEffectKind;
use else_protocol::harness::{
    CancelSupport, HarnessCapabilities, ToolEffectVisibility, UsageSupport,
};
use else_protocol::resource::{ModelUsage, UsageBasis};

use crate::adapter::{
    parse_plan_draft, shell_quote, BridgeEvent, HarnessPrep, LineParser, ProcessHarness,
};

pub const CODEX: &str = "codex";
pub const CLAUDE_CODE: &str = "claude-code";

/// 真实 Harness 的墙钟超时：到点杀进程判失败，不等待其自觉。
const REAL_HARNESS_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15 * 60);

/// Codex CLI（codex exec --json）。不支持优雅取消（KillOnly），
/// 工具副作用只能观察（Observable），用量只报 Token 不报费用（Estimated 混合证据）。
pub fn codex() -> ProcessHarness<CodexParser> {
    ProcessHarness {
        name: CODEX,
        capabilities: HarnessCapabilities {
            cancel: CancelSupport::KillOnly,
            checkpoint_resume: true, // codex exec resume，else 不依赖它做权威恢复
            usage: UsageSupport::Estimated,
            tool_effects: ToolEffectVisibility::Opaque,
            interactive_input: false,
        },
        network: NetworkPolicy::AllowOutbound,
        timeout: Some(REAL_HARNESS_TIMEOUT),
        prepare: Box::new(|_workspace, private_dir, prompt| {
            let binary = resolve_binary(
                "codex",
                &[std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/bin/codex"))],
            )?;
            let (home, cleanup_root) = stage_private_dir(private_dir, "codex-home")?;
            if let Some(source) = host_home().map(|home| home.join(".codex")) {
                // config.toml 可能相对引用同目录文件（如 model_catalog_json），一并暂存。
                for file in [
                    "auth.json",
                    "config.toml",
                    "cc-switch-model-catalog.json",
                    "version.json",
                ] {
                    let from = source.join(file);
                    if from.is_file() {
                        std::fs::copy(&from, home.join(file))?;
                    }
                }
            }
            let mut env = vec![("CODEX_HOME".to_string(), home.display().to_string())];
            env.extend(host_env(&["OPENAI_API_KEY", "OPENAI_BASE_URL"]));
            Ok(HarnessPrep {
                // else 的 Sandbox 是第一道墙；Codex 自带的嵌套沙箱（seatbelt 套
                // seatbelt）无法工作，显式降级并由 else 的 Profile 接管隔离。
                // notify 指向宿主 GUI 程序，沙箱内不可用，显式清空。
                command: format!(
                    "{} exec --json --skip-git-repo-check \
                     -c sandbox_mode=\"danger-full-access\" -c 'notify=[]' {}",
                    shell_quote(&binary.display().to_string()),
                    shell_quote(prompt)
                ),
                env,
                // 二进制本身与其所在目录（codex 会列举自身 bin 目录）；
                // /etc/codex（managed requirements）在本机不存在：沙箱默认拒绝会让
                // codex 把 ENOENT 误判为 EPERM 而启动失败。/etc 本身为世界可读的
                // 系统配置，只读放行使其看到真实的"不存在"；不放行任何写入。
                read_paths: [
                    binary.clone(),
                    binary.parent().map(Path::to_path_buf).unwrap_or_default(),
                    PathBuf::from("/etc"),
                ]
                .into_iter()
                .filter(|path| !path.as_os_str().is_empty())
                .collect(),
                write_paths: vec![home],
                cleanup_paths: vec![cleanup_root],
            })
        }),
        parser: CodexParser,
    }
}

/// Claude Code CLI（claude -p --output-format stream-json）。
/// 文件编辑自动接受（acceptEdits），任何需要提示的动作自动拒绝
/// （permission-prompts none）：特权动作走 else 的 Decision 通道，不由 CLI 自行询问。
pub fn claude_code() -> ProcessHarness<ClaudeParser> {
    ProcessHarness {
        name: CLAUDE_CODE,
        capabilities: HarnessCapabilities {
            cancel: CancelSupport::KillOnly,
            checkpoint_resume: true, // claude --resume，else 不依赖它做权威恢复
            usage: UsageSupport::Reported,
            tool_effects: ToolEffectVisibility::Opaque,
            interactive_input: false,
        },
        network: NetworkPolicy::AllowOutbound,
        timeout: Some(REAL_HARNESS_TIMEOUT),
        prepare: Box::new(|_workspace, private_dir, prompt| {
            let binary = resolve_binary("claude", &[])?;
            let (home, cleanup_root) = stage_private_dir(private_dir, "claude-config")?;
            let mut env = vec![
                ("CLAUDE_CONFIG_DIR".to_string(), home.display().to_string()),
                // claude 2.x 是 bun 编译的原生二进制；PATH 兜底其可能调用的子工具
                (
                    "PATH".to_string(),
                    "/opt/homebrew/bin:/usr/bin:/bin:/usr/sbin:/sbin".to_string(),
                ),
            ];
            env.extend(host_env(&[
                "ANTHROPIC_API_KEY",
                "ANTHROPIC_AUTH_TOKEN",
                "ANTHROPIC_BASE_URL",
                "ANTHROPIC_MODEL",
            ]));
            Ok(HarnessPrep {
                command: format!(
                    "{} -p {} --output-format stream-json --verbose \
                     --permission-mode acceptEdits --permission-prompts none",
                    shell_quote(&binary.display().to_string()),
                    shell_quote(prompt)
                ),
                env,
                // claude 2.x 为 bun 编译的单文件原生二进制，canonicalize 后只读放行
                // 其真实路径即可；若未来拆分运行时再按拒绝日志收敛。
                read_paths: vec![binary],
                write_paths: vec![home],
                cleanup_paths: vec![cleanup_root],
            })
        }),
        parser: ClaudeParser,
    }
}

/// 按 PATH 与常见位置解析 Harness 二进制绝对路径；找不到是明确的启动失败。
fn resolve_binary(name: &str, extra: &[Option<PathBuf>]) -> std::io::Result<PathBuf> {
    let mut candidates: Vec<PathBuf> = extra.iter().flatten().cloned().collect();
    if let Some(path) = std::env::var_os("PATH") {
        candidates.extend(std::env::split_paths(&path).map(|dir| dir.join(name)));
    }
    candidates
        .iter()
        .find(|path| path.is_file())
        .map(|path| path.canonicalize().unwrap_or_else(|_| path.clone()))
        .ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("harness binary {name} not found on PATH"),
            )
        })
}

fn stage_private_dir(private_dir: &Path, name: &str) -> std::io::Result<(PathBuf, PathBuf)> {
    let root = private_dir.join(format!("run-{}", uuid::Uuid::new_v4()));
    let dir = root.join(name);
    std::fs::create_dir_all(&dir)?;
    Ok((dir, root))
}

fn host_home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

fn host_env(keys: &[&str]) -> Vec<(String, String)> {
    keys.iter()
        .filter_map(|key| {
            std::env::var(key)
                .ok()
                .map(|value| (key.to_string(), value))
        })
        .collect()
}

/// 读操作工具白名单；写文件收敛到工作区故为 Reversible；其余一律 Unknown 保守记账。
fn claude_tool_effect(tool: &str) -> SideEffectKind {
    match tool {
        "Read" | "Glob" | "Grep" | "LS" | "WebFetch" | "WebSearch" => SideEffectKind::ReadOnly,
        "Write" | "Edit" | "NotebookEdit" => SideEffectKind::Reversible,
        _ => SideEffectKind::Unknown,
    }
}

/// Codex `codex exec --json` 行解析器（JSONL；非 JSON 行如进度提示跳过）。
pub struct CodexParser;

impl LineParser for CodexParser {
    fn parse(&self, line: &str) -> Vec<BridgeEvent> {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
            return vec![];
        };
        let event = match value["type"].as_str() {
            Some("item.completed") => {
                let item = &value["item"];
                match item["type"].as_str() {
                    Some("agent_message") => {
                        let text = item["text"].as_str().unwrap_or_default();
                        Some(match parse_plan_draft(text) {
                            Some(draft) => BridgeEvent::PlanProposed { draft },
                            None => BridgeEvent::Output { text: text.into() },
                        })
                    }
                    // 配置告警等非致命错误也走 error item：只记录为输出，不判失败。
                    Some("error") => Some(BridgeEvent::Output {
                        text: format!("codex: {}", item["message"].as_str().unwrap_or("error")),
                    }),
                    Some("command_execution") => Some(BridgeEvent::ToolObserved {
                        tool: "shell".into(),
                        effect: SideEffectKind::Unknown,
                        summary: item["command"].as_str().unwrap_or("").to_string(),
                        success: item["exit_code"]
                            .as_i64()
                            .map(|code| code == 0)
                            .unwrap_or(true),
                    }),
                    Some("file_change") => {
                        let path = item["changes"]
                            .as_array()
                            .and_then(|changes| changes.first())
                            .and_then(|change| change["path"].as_str())
                            .unwrap_or("workspace")
                            .to_string();
                        Some(BridgeEvent::Artifact {
                            locator: path,
                            note: "codex file_change".into(),
                        })
                    }
                    _ => None,
                }
            }
            Some("turn.completed") => {
                // 用量缺失不能记成零成本：Unknown + 保守预算。
                let usage = if value["usage"].is_object() {
                    let usage = &value["usage"];
                    ModelUsage {
                        input_tokens: usage["input_tokens"].as_u64().unwrap_or(0),
                        output_tokens: usage["output_tokens"].as_u64().unwrap_or(0),
                        // Codex 不回报费用：Token 实测、费用缺测，混合证据记 Estimated。
                        cost_usd_micros: 0,
                        basis: UsageBasis::Estimated,
                    }
                } else {
                    ModelUsage::unknown()
                };
                return vec![
                    BridgeEvent::Usage {
                        model: "codex".into(),
                        usage,
                    },
                    // turn.completed 是 Codex 的成功终局信号
                    BridgeEvent::Completed {
                        summary: String::new(),
                    },
                ];
            }
            Some("turn.failed") => Some(BridgeEvent::Failed {
                reason: value["error"]["message"]
                    .as_str()
                    .unwrap_or("turn failed")
                    .to_string(),
            }),
            Some("error") => Some(BridgeEvent::Failed {
                reason: value["message"]
                    .as_str()
                    .unwrap_or("codex error")
                    .to_string(),
            }),
            _ => None,
        };
        event.into_iter().collect()
    }
}

/// Claude Code `--output-format stream-json` 行解析器。
pub struct ClaudeParser;

impl LineParser for ClaudeParser {
    fn parse(&self, line: &str) -> Vec<BridgeEvent> {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
            return vec![];
        };
        match value["type"].as_str() {
            Some("assistant") => value["message"]["content"]
                .as_array()
                .map(|content| {
                    content
                        .iter()
                        .filter_map(|block| match block["type"].as_str() {
                            Some("text") => {
                                let text = block["text"].as_str().unwrap_or_default();
                                Some(match parse_plan_draft(text) {
                                    Some(draft) => BridgeEvent::PlanProposed { draft },
                                    None => BridgeEvent::Output { text: text.into() },
                                })
                            }
                            Some("tool_use") => {
                                let tool = block["name"].as_str().unwrap_or("unknown");
                                Some(BridgeEvent::ToolObserved {
                                    tool: tool.to_string(),
                                    effect: claude_tool_effect(tool),
                                    summary: summarize_input(&block["input"]),
                                    success: true,
                                })
                            }
                            _ => None,
                        })
                        .collect()
                })
                .unwrap_or_default(),
            Some("system") if value["subtype"].as_str() == Some("api_retry") => {
                vec![BridgeEvent::Output {
                    text: format!(
                        "claude: API 重试 {}/{}（{}）",
                        value["attempt"].as_u64().unwrap_or(0),
                        value["max_retries"].as_u64().unwrap_or(0),
                        value["error"].as_str().unwrap_or("unknown")
                    ),
                }]
            }
            Some("result") => {
                let model = value["modelUsage"]
                    .as_object()
                    .and_then(|usage| usage.keys().next().cloned())
                    .unwrap_or_else(|| "claude".into());
                // 用量缺失不能记成零成本：Unknown + 保守预算。
                let usage = if value["usage"].is_object() {
                    let usage = &value["usage"];
                    ModelUsage {
                        input_tokens: usage["input_tokens"].as_u64().unwrap_or(0)
                            + usage["cache_read_input_tokens"].as_u64().unwrap_or(0)
                            + usage["cache_creation_input_tokens"].as_u64().unwrap_or(0),
                        output_tokens: usage["output_tokens"].as_u64().unwrap_or(0),
                        cost_usd_micros: (value["total_cost_usd"].as_f64().unwrap_or(0.0)
                            * 1_000_000.0)
                            .round() as u64,
                        basis: UsageBasis::Reported,
                    }
                } else {
                    ModelUsage::unknown()
                };
                let usage = BridgeEvent::Usage { model, usage };
                let terminal = if value["is_error"].as_bool().unwrap_or(false) {
                    BridgeEvent::Failed {
                        reason: value["result"]
                            .as_str()
                            .unwrap_or("claude error")
                            .to_string(),
                    }
                } else {
                    BridgeEvent::Completed {
                        summary: value["result"].as_str().unwrap_or_default().to_string(),
                    }
                };
                vec![usage, terminal]
            }
            _ => vec![],
        }
    }
}

fn summarize_input(input: &serde_json::Value) -> String {
    for key in ["command", "file_path", "path", "pattern", "url", "prompt"] {
        if let Some(value) = input[key].as_str() {
            return value.chars().take(200).collect();
        }
    }
    input.to_string().chars().take(200).collect()
}
