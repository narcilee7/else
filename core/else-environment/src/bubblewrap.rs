use std::path::{Path, PathBuf};
use std::process::Command;

use crate::NetworkPolicy;

pub(super) fn command(
    executable: &Path,
    workspace: &Path,
    shell_command: &str,
    network: NetworkPolicy,
    read_paths: &[PathBuf],
    write_paths: &[PathBuf],
    workspace_writable: bool,
) -> Command {
    let mut command = Command::new(executable);
    command
        .arg("--die-with-parent")
        .arg("--new-session")
        // 包含独立网络命名空间：没有出口网络，也不依赖宿主机防火墙。
        .arg("--unshare-all");
    // 显式放通出站（Harness 原生 Provider 流量）：与宿主共享网络命名空间。
    if network == NetworkPolicy::AllowOutbound {
        command.arg("--share-net");
    }
    command
        .arg("--clearenv")
        .arg("--proc")
        .arg("/proc")
        .arg("--dev")
        .arg("/dev");
    // 仅提供运行二进制所需只读系统路径；不绑定 /etc、HOME 或宿主 Workspace 外目录。
    for path in ["/usr", "/bin", "/sbin", "/lib", "/lib64"] {
        if Path::new(path).exists() {
            command.arg("--ro-bind").arg(path).arg(path);
        }
    }
    // 额外只读路径（Harness 二进制与运行时依赖）。
    for path in read_paths {
        let path = path.canonicalize().unwrap_or_else(|_| path.clone());
        if path.exists() {
            command.arg("--ro-bind").arg(&path).arg(&path);
        }
    }
    for path in write_paths {
        let path = path.canonicalize().unwrap_or_else(|_| path.clone());
        if path.exists() {
            command.arg("--bind").arg(&path).arg(&path);
        }
    }
    command
        .arg(if workspace_writable {
            "--bind"
        } else {
            "--ro-bind"
        })
        .arg(workspace)
        .arg(workspace)
        .arg("--chdir")
        .arg(workspace)
        .arg("--setenv")
        .arg("HOME")
        .arg(workspace)
        .arg("--setenv")
        .arg("TMPDIR")
        .arg(workspace)
        .arg("--setenv")
        .arg("PATH")
        .arg("/usr/bin:/bin:/usr/sbin:/sbin")
        .arg("--setenv")
        .arg("LC_ALL")
        .arg("C")
        .arg("/bin/sh")
        .arg("-c")
        .arg(shell_command);
    command
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_unshares_network_and_binds_only_workspace_writable() {
        let command = command(
            Path::new("/usr/bin/bwrap"),
            Path::new("/tmp/workspace"),
            "echo ok",
            NetworkPolicy::Deny,
            &[],
            &[],
            true,
        );
        let args = command
            .get_args()
            .map(|arg| arg.to_string_lossy())
            .collect::<Vec<_>>();
        assert!(args.iter().any(|arg| arg == "--unshare-all"));
        assert!(args.iter().any(|arg| arg == "--die-with-parent"));
        assert!(!args.iter().any(|arg| arg == "--share-net"));
        assert!(!args.iter().any(|arg| arg == "/etc"));
        let bind = args
            .windows(3)
            .find(|args| args[0] == "--bind")
            .expect("one writable workspace binding");
        assert_eq!(bind[1], "/tmp/workspace");
        assert_eq!(bind[2], "/tmp/workspace");
    }

    #[test]
    fn allow_outbound_shares_host_network() {
        let command = command(
            Path::new("/usr/bin/bwrap"),
            Path::new("/tmp/workspace"),
            "echo ok",
            NetworkPolicy::AllowOutbound,
            &[],
            &[],
            true,
        );
        let args = command
            .get_args()
            .map(|arg| arg.to_string_lossy())
            .collect::<Vec<_>>();
        assert!(args.iter().any(|arg| arg == "--share-net"));
    }
}
