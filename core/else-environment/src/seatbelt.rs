use std::path::{Path, PathBuf};
use std::process::Command;

use crate::{NetworkPolicy, Result, SandboxError};

pub(super) fn command(
    executable: &Path,
    workspace: &Path,
    shell_command: &str,
    network: NetworkPolicy,
    read_paths: &[PathBuf],
    write_paths: &[PathBuf],
    workspace_writable: bool,
) -> Result<Command> {
    let profile = profile(
        workspace,
        network,
        read_paths,
        write_paths,
        workspace_writable,
    )?;
    let mut command = Command::new(executable);
    command
        .arg("-p")
        .arg(profile)
        .arg("/bin/sh")
        .arg("-c")
        .arg(shell_command);
    Ok(command)
}

fn profile(
    workspace: &Path,
    network: NetworkPolicy,
    read_paths: &[PathBuf],
    write_paths: &[PathBuf],
    workspace_writable: bool,
) -> Result<String> {
    let workspace = workspace
        .to_str()
        .ok_or_else(|| SandboxError::InvalidProfile("workspace path must be UTF-8".into()))?;
    let mut metadata_paths: Vec<PathBuf> = Path::new(workspace)
        .ancestors()
        .map(Path::to_path_buf)
        .collect();
    let network_rule = match network {
        NetworkPolicy::Deny => {
            "; 没有 network-inbound/network-outbound 规则：网络默认拒绝。".to_string()
        }
        NetworkPolicy::Proxy {
            port,
            tool_gateway_port,
        } => {
            let mut rule = format!("(allow network-outbound (remote ip \"localhost:{port}\"))");
            if let Some(tool_port) = tool_gateway_port {
                rule.push_str(&format!(
                    "\n(allow network-outbound (remote ip \"localhost:{tool_port}\"))"
                ));
            }
            rule
        }
        // 显式放通全部出站（Harness 原生 Provider 流量）；入站仍默认拒绝。
        NetworkPolicy::AllowOutbound => "(allow network-outbound)".to_string(),
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
    let mut writes = Vec::new();
    metadata_paths.extend(write_paths.iter().cloned());
    for path in write_paths {
        let canonical = path.canonicalize().unwrap_or_else(|_| path.clone());
        for path in [path.clone(), canonical] {
            if let Some(path) = path.to_str() {
                let rule = format!("(subpath {})", quoted(path));
                if !writes.contains(&rule) {
                    writes.push(rule);
                }
            }
        }
        if let Some(parent) = path.parent() {
            metadata_paths.extend(parent.ancestors().map(Path::to_path_buf));
        }
    }
    let extra_writes = writes.join(" ");
    let mut metadata = Vec::new();
    // firmlink 两边都要：路径写作 `/tmp/...` 时，`canonicalize()` 会得到
    // `/private/tmp/...`，而 Seatbelt 按字面匹配。只放原始拼写会让
    // 子进程的 canonicalize 返回 EPERM（Codex 就是这样挂的）。
    let mut all_metadata = metadata_paths.clone();
    all_metadata.extend(
        metadata_paths
            .iter()
            .filter_map(|path| path.canonicalize().ok()),
    );
    for path in all_metadata {
        if let Some(path) = path.to_str() {
            let rule = format!("(literal {})", quoted(path));
            if !metadata.contains(&rule) {
                metadata.push(rule);
            }
        }
    }
    let metadata_paths = metadata.join(" ");
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
(allow file-read-metadata {metadata_paths})
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
    {extra_writes}
    (subpath {workspace}))
(allow file-write*
    (literal "/dev/null")
    {workspace_write}
    {extra_writes})
{network_rule}
"#,
        workspace = quoted(workspace),
        network_rule = network_rule,
        extra_reads = extra_reads,
        metadata_paths = metadata_paths,
        extra_writes = extra_writes,
        workspace_write = if workspace_writable {
            format!("(subpath {})", quoted(workspace))
        } else {
            String::new()
        },
    ))
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
            &[],
            true,
        )
        .unwrap();
        assert!(profile.contains("(deny default)"));
        assert!(!profile.contains("(allow network"));
        assert!(profile.contains("\\\"quoted\\\""));
        assert!(profile.contains("(allow file-write*"));
    }

    #[test]
    fn allow_outbound_is_explicit_and_keeps_inbound_denied() {
        let profile = profile(
            Path::new("/tmp/ws"),
            NetworkPolicy::AllowOutbound,
            &[],
            &[],
            true,
        )
        .unwrap();
        assert!(profile.contains("(allow network-outbound)"));
        assert!(!profile.contains("(allow network-inbound"));
        assert!(!profile.contains("(allow network)"));
    }

    #[test]
    fn proxy_network_allows_only_loopback_proxy_port() {
        let profile = profile(
            Path::new("/tmp/ws"),
            NetworkPolicy::Proxy {
                port: 43123,
                tool_gateway_port: Some(43124),
            },
            &[],
            &[],
            true,
        )
        .unwrap();
        assert!(profile.contains(r#"(remote ip "localhost:43123")"#));
        assert!(profile.contains(r#"(remote ip "localhost:43124")"#));
        assert!(!profile.contains("(allow network-outbound)\n"));
        assert!(!profile.contains("(allow network-inbound"));
    }

    #[test]
    fn extra_read_paths_are_read_only() {
        let profile = profile(
            Path::new("/tmp/ws"),
            NetworkPolicy::Deny,
            &[PathBuf::from("/opt/homebrew")],
            &[],
            true,
        )
        .unwrap();
        assert!(profile.contains("(subpath \"/opt/homebrew\")"));
        // 只读：写入段只放行 /dev/null 与工作区
        let write_section = profile.split("(allow file-write*").nth(1).unwrap();
        assert!(!write_section.contains("homebrew"));
    }

    #[test]
    fn profile_allows_framework_shm_and_var_metadata() {
        let profile = profile(Path::new("/tmp/ws"), NetworkPolicy::Deny, &[], &[], true).unwrap();
        // CFPreferences 同步所需的共享内存（单反斜杠正则，双写会变成字面反斜杠）
        assert!(profile.contains(r#"(ipc-posix-name-regex #"apple\..*")"#));
        // libinfo 初始化需要 stat /var，firmlink 双形并列
        assert!(profile.contains(r#"(literal "/var") (literal "/private/var")"#));
    }

    #[test]
    fn read_only_workspace_omits_workspace_write_rule() {
        let profile = profile(Path::new("/tmp/ws"), NetworkPolicy::Deny, &[], &[], false).unwrap();
        let write_section = profile.split("(allow file-write*").nth(1).unwrap();
        assert!(!write_section.contains("(subpath \"/tmp/ws\")"));
    }
}
