use std::path::Path;
use std::process::Command;

use crate::{Result, SandboxError};

pub(super) fn command(executable: &Path, workspace: &Path, shell_command: &str) -> Result<Command> {
    let profile = profile(workspace)?;
    let mut command = Command::new(executable);
    command
        .arg("-p")
        .arg(profile)
        .arg("/bin/sh")
        .arg("-c")
        .arg(shell_command);
    Ok(command)
}

fn profile(workspace: &Path) -> Result<String> {
    let workspace = workspace
        .to_str()
        .ok_or_else(|| SandboxError::InvalidProfile("workspace path must be UTF-8".into()))?;
    let ancestors = workspace_ancestors(Path::new(workspace));
    Ok(format!(
        r#"(version 1)
(deny default)
(allow process-fork)
(allow process-exec)
(allow signal (target same-sandbox))
(allow mach-lookup)
(allow sysctl-read)
; dyld 与 getcwd 需要根目录和 Workspace 祖先的路径元数据，不开放其文件内容。
(allow file-read-data (literal "/"))
(allow file-read-metadata {ancestors})
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
    (subpath {workspace}))
(allow file-write*
    (literal "/dev/null")
    (subpath {workspace}))
; 没有 network-inbound/network-outbound 规则：网络默认拒绝。
"#,
        ancestors = ancestors,
        workspace = quoted(workspace),
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
        let profile = profile(Path::new("/tmp/work space/\"quoted\"")).unwrap();
        assert!(profile.contains("(deny default)"));
        assert!(!profile.contains("(allow network"));
        assert!(profile.contains("\\\"quoted\\\""));
        assert!(profile.contains("(allow file-write*"));
    }
}
