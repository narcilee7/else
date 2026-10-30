use std::path::{Path, PathBuf};
use std::process::Command;

use crate::{NetworkPolicy, Result, SandboxError};

pub(super) fn command(
    executable: &Path,
    workspace: &Path,
    shell_command: &str,
    network: NetworkPolicy,
    read_paths: &[PathBuf],
) -> Result<Command> {
    let profile = profile(workspace, network, read_paths)?;
    let mut command = Command::new(executable);
    command
        .arg("-p")
        .arg(profile)
        .arg("/bin/sh")
        .arg("-c")
        .arg(shell_command);
    Ok(command)
}

fn profile(workspace: &Path, network: NetworkPolicy, read_paths: &[PathBuf]) -> Result<String> {
    let workspace = workspace
        .to_str()
        .ok_or_else(|| SandboxError::InvalidProfile("workspace path must be UTF-8".into()))?;
    let ancestors = workspace_ancestors(Path::new(workspace));
    let network_rule = match network {
        NetworkPolicy::Deny => "; 没有 network-inbound/network-outbound 规则：网络默认拒绝。",
        // 显式放通全部出站（Harness 原生 Provider 流量）；入站仍默认拒绝。
        NetworkPolicy::AllowOutbound => "(allow network-outbound)",
    };
    // 额外只读路径（Harness 二进制与运行时依赖）：只读，不可写。
    // macOS 的 /etc、/tmp、/var 是 firmlink 而非普通符号链接，Seatbelt 按
    // 访问路径字面匹配：原始路径与解析后路径必须同时放行。
    let mut extra: Vec<String> = Vec::new();
    for path in read_paths {
        let canonical = path.canonicalize().unwrap_or_else(|_| path.clone());
        for path in [path.clone(), canonical] {
            if let Some(path) = path.to_str() {
                let rule = format!("(subpath {})", quoted(path));
                if !extra.contains(&rule) {
                    extra.push(rule);
                }
            }
        }
    }
    let extra_reads = extra.join(" ");
    Ok(format!(
        r#"(version 1)
(deny default)
(allow process-fork)
(allow process-exec)
(allow signal (target same-sandbox))
(allow mach-lookup)
(allow sysctl-read)
; macOS 系统框架（CFPreferences/通知中心等）需要读 apple.* 共享内存段；
; 只读放行，不写。缺少这条时真实 Harness（如 Codex）的配置同步会被拒绝。
(allow ipc-posix-shm-read-data (ipc-posix-name-regex #"apple\..*"))
; dyld 与 getcwd 需要根目录和 Workspace 祖先的路径元数据，不开放其文件内容。
(allow file-read-data (literal "/"))
(allow file-read-metadata {ancestors})
; 系统框架（libinfo/Network）初始化要 stat /var；拒绝会导致 DNS 整体失败。
; firmlink 字面匹配：/var 与 /private/var 必须同时列出。只放元数据，不放内容。
(allow file-read-metadata (literal "/var") (literal "/private/var"))
(allow file-read*
    (subpath "/bin")
    (subpath "/sbin")
    (subpath "/usr/bin")
    (subpath "/usr/sbin")
    (subpath "/usr/lib")
    (subpath "/usr/libexec")
    (subpath "/usr/share")
    (subpath "/System")
    (subpath "/dev")
    (subpath "/private/var/select")
    (subpath "/var/select")
    {extra_reads}
    (subpath {workspace}))
(allow file-write*
    (literal "/dev/null")
    (subpath {workspace}))
{network_rule}
"#,
        ancestors = ancestors,
        workspace = quoted(workspace),
        network_rule = network_rule,
        extra_reads = extra_reads,
    ))
}

fn workspace_ancestors(workspace: &Path) -> String {
    workspace
        .ancestors()
        .filter(|path| *path != workspace)
        .map(|path| format!("(literal {})", quoted(&path.to_string_lossy())))
        .collect::<Vec<_>>()
        .join(" ")
}

fn quoted(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_is_deny_by_default_and_has_no_network_allowance() {
        let profile = profile(
            Path::new("/tmp/work space/\"quoted\""),
            NetworkPolicy::Deny,
            &[],
        )
        .unwrap();
        assert!(profile.contains("(deny default)"));
        assert!(!profile.contains("(allow network"));
        assert!(profile.contains("\\\"quoted\\\""));
        assert!(profile.contains("(allow file-write*"));
    }

    #[test]
    fn allow_outbound_is_explicit_and_keeps_inbound_denied() {
        let profile = profile(Path::new("/tmp/ws"), NetworkPolicy::AllowOutbound, &[]).unwrap();
        assert!(profile.contains("(allow network-outbound)"));
        assert!(!profile.contains("(allow network-inbound"));
        assert!(!profile.contains("(allow network)"));
    }

    #[test]
    fn extra_read_paths_are_read_only() {
        let profile = profile(
            Path::new("/tmp/ws"),
            NetworkPolicy::Deny,
            &[PathBuf::from("/opt/homebrew")],
        )
        .unwrap();
        assert!(profile.contains("(subpath \"/opt/homebrew\")"));
        // 只读：写入段只放行 /dev/null 与工作区
        let write_section = profile.split("(allow file-write*").nth(1).unwrap();
        assert!(!write_section.contains("homebrew"));
    }

    #[test]
    fn profile_allows_framework_shm_and_var_metadata() {
        let profile = profile(Path::new("/tmp/ws"), NetworkPolicy::Deny, &[]).unwrap();
        // CFPreferences 同步所需的共享内存（单反斜杠正则，双写会变成字面反斜杠）
        assert!(profile.contains(r#"(ipc-posix-name-regex #"apple\..*")"#));
        // libinfo 初始化需要 stat /var，firmlink 双形并列
        assert!(profile.contains(r#"(literal "/var") (literal "/private/var")"#));
    }
}
