//! 后台 Runtime 服务的注册。
//!
//! else 的 Runtime 应该**常驻**：Connector 要按计划同步，通知要按时送达，
//! 终端与 App 要看到同一份世界。一个只在终端里活着的 daemon 做不到这些。
//!
//! 注册方式按平台走：
//!
//! - macOS：LaunchAgent（`~/Library/LaunchAgents`）。
//! - Linux：systemd user unit（`~/.config/systemd/user`）。
//!
//! 两者都注册到**同一个 runtime 目录**，并且都用「一个机器一个 daemon」的
//! 约定：macOS 上与 App 自带的 LaunchAgent 同名，谁先装谁负责。

use std::path::{Path, PathBuf};

use crate::error::{CliError, Result};

/// macOS 上 daemon 的 LaunchAgent 标签。
///
/// 与 macOS App 自带的 helper **同名**：一个机器只有一个 Runtime，
/// 两个都注册会互相抢同一个账本。
#[cfg(target_os = "macos")]
pub const LAUNCH_AGENT_LABEL: &str = "com.else.desktop.runtime";

/// Linux 上 daemon 的 systemd user unit 名。
#[cfg(target_os = "linux")]
pub const SYSTEMD_UNIT: &str = "else.service";

/// 后台服务状态。
#[derive(Debug, Clone, serde::Serialize)]
pub struct ServiceStatus {
    /// 当前平台。
    pub platform: &'static str,
    /// 服务定义文件的位置。
    pub unit_path: Option<PathBuf>,
    /// 定义文件是否在。
    pub installed: bool,
    /// 服务是否已经被系统加载（已加载不等于在跑）。
    pub loaded: bool,
    /// 进程是否真的活着。`launchctl` 会列出已退出、正在重试的 job，
    /// 只看 label 会把一个崩溃循环当成健康。
    pub running: bool,
    /// 服务定义实际指向的 runtime 目录（从 unit 文件读出来，不是猜的）。
    pub unit_runtime_dir: Option<PathBuf>,
    /// 一句人话说明。
    pub detail: String,
}

/// 查后台服务状态。
pub fn status(runtime_dir: &Path) -> ServiceStatus {
    #[cfg(target_os = "macos")]
    {
        macos_status(runtime_dir)
    }
    #[cfg(target_os = "linux")]
    {
        linux_status(runtime_dir)
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        unsupported_status()
    }
}

/// 注册后台服务。
pub fn install(runtime_dir: &Path) -> Result<ServiceStatus> {
    #[cfg(target_os = "macos")]
    {
        macos_install(runtime_dir)
    }
    #[cfg(target_os = "linux")]
    {
        linux_install(runtime_dir)
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        Err(CliError::usage(
            "这个平台还没有后台服务支持；可以用 `else-bot serve` 前台运行。",
        ))
    }
}

/// 注销后台服务。
pub fn uninstall(runtime_dir: &Path) -> Result<ServiceStatus> {
    #[cfg(target_os = "macos")]
    {
        macos_uninstall(runtime_dir)
    }
    #[cfg(target_os = "linux")]
    {
        linux_uninstall(runtime_dir)
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        Err(CliError::usage(
            "这个平台还没有后台服务支持；可以用 `else-bot serve` 前台运行。",
        ))
    }
}

/// 没有后台服务支持的平台。
#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn unsupported_status() -> ServiceStatus {
    ServiceStatus {
        platform: "unsupported",
        unit_path: None,
        installed: false,
        loaded: false,
        running: false,
        unit_runtime_dir: None,
        detail: "这个平台还没有后台服务支持；可以用 `else-bot serve` 前台运行。".into(),
    }
}

/// 当前可执行文件路径。
fn current_exe() -> Result<PathBuf> {
    std::env::current_exe()
        .map_err(|error| CliError::usage(format!("cannot locate the else binary: {error}")))
}

/// macOS 的 LaunchAgent 路径。
#[cfg(target_os = "macos")]
fn macos_unit_path() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    Some(
        PathBuf::from(home)
            .join("Library")
            .join("LaunchAgents")
            .join(format!("{LAUNCH_AGENT_LABEL}.plist")),
    )
}

/// Linux 的 systemd user unit 路径。
#[cfg(target_os = "linux")]
fn linux_unit_path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))?;
    Some(base.join("systemd").join("user").join(SYSTEMD_UNIT))
}

#[cfg(target_os = "macos")]
fn macos_status(runtime_dir: &Path) -> ServiceStatus {
    let unit_path = macos_unit_path();
    let installed = unit_path.as_ref().is_some_and(|path| path.exists());
    let job = launchctl("list")
        .ok()
        .and_then(|output| parse_launchctl_job(&output, LAUNCH_AGENT_LABEL));
    let loaded = job.is_some();
    let running = job.as_ref().is_some_and(|job| job.running);
    // 读 unit 文件里真实的 `--dir`，而不是把手上的目录拼到描述里。
    let unit_runtime_dir = unit_path
        .as_ref()
        .and_then(|path| std::fs::read_to_string(path).ok())
        .and_then(|body| plist_argument(&body, "--dir"));
    let detail = if running {
        format!("LaunchAgent {LAUNCH_AGENT_LABEL} 正在运行")
    } else if let Some(job) = job.filter(|job| !job.running) {
        format!(
            "LaunchAgent {LAUNCH_AGENT_LABEL} 已加载但已退出（退出码 {:?}）；\
             它在反复重启，日志见 ~/Library/Logs/else-runtime.log",
            job.last_exit
        )
    } else if installed {
        "LaunchAgent 已安装但没有加载；运行 `else-bot daemon install` 加载它".into()
    } else {
        "没有注册后台服务；运行 `else-bot daemon install`".into()
    };
    ServiceStatus {
        platform: "macos",
        unit_path,
        installed,
        loaded,
        running,
        unit_runtime_dir,
        detail,
    }
    .with_runtime(runtime_dir)
}

/// `launchctl list` 的一行：`PID\tStatus\tLabel`。
/// PID 为 `-` 表示 job 已加载但进程没在跑。
#[cfg(target_os = "macos")]
struct LaunchctlJob {
    running: bool,
    last_exit: Option<i32>,
}

#[cfg(target_os = "macos")]
fn parse_launchctl_job(output: &str, label: &str) -> Option<LaunchctlJob> {
    output.lines().find_map(|line| {
        let mut columns = line.split('\t');
        let pid = columns.next()?.trim();
        let status = columns.next()?.trim();
        let name = columns.next()?.trim();
        (name == label).then(|| LaunchctlJob {
            running: pid != "-" && !pid.is_empty(),
            last_exit: status.parse::<i32>().ok(),
        })
    })
}

/// 从 plist 文本里取 `--dir` 后面那个参数。
#[cfg(target_os = "macos")]
fn plist_argument(body: &str, flag: &str) -> Option<PathBuf> {
    let mut values = body
        .split("<string>")
        .filter_map(|chunk| chunk.split("</string>").next());
    let mut previous: Option<&str> = None;
    for value in values.by_ref() {
        if previous == Some(flag) {
            return Some(PathBuf::from(value.trim()));
        }
        previous = Some(value.trim());
    }
    None
}

/// macOS 后台服务的日志文件。
///
/// 没有它，一个崩溃循环的 LaunchAgent 只会默默重试：`launchctl` 说"已加载"，
/// 而用户看不到任何原因。
#[cfg(target_os = "macos")]
pub fn launch_log_path() -> Option<PathBuf> {
    std::env::var_os("HOME").map(|home| {
        PathBuf::from(home)
            .join("Library")
            .join("Logs")
            .join("else-runtime.log")
    })
}

/// macOS LaunchAgent 的内容。
///
/// 单独抽出来是为了能直接检查它长什么样 —— 这个文件决定了 daemon 能不能
/// 在登录后自动起来，出了问题很难从别处看出来。
#[cfg(target_os = "macos")]
fn macos_plist(program: &Path, runtime_dir: &Path) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \
         \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
         <plist version=\"1.0\">\n\
         <dict>\n\
         \x20 <key>Label</key>\n\
         \x20 <string>{LAUNCH_AGENT_LABEL}</string>\n\
         \x20 <key>ProgramArguments</key>\n\
         \x20 <array>\n\
         \x20   <string>{}</string>\n\
         \x20   <string>--dir</string>\n\
         \x20   <string>{}</string>\n\
         \x20   <string>serve</string>\n\
         \x20 </array>\n\
         \x20 <key>RunAtLoad</key>\n\
         \x20 <true/>\n\
         \x20 <key>KeepAlive</key>\n\
         \x20 <true/>\n\
         \x20 <key>ProcessType</key>\n\
         \x20 <string>Interactive</string>\n\
         \x20 <key>ThrottleInterval</key>\n\
         \x20 <integer>10</integer>\n\
         \x20 <key>StandardOutPath</key>\n\
         \x20 <string>{}</string>\n\
         \x20 <key>StandardErrorPath</key>\n\
         \x20 <string>{}</string>\n\
         </dict>\n\
         </plist>\n",
        program.display(),
        runtime_dir.display(),
        launch_log_path()
            .map(|path| path.display().to_string())
            .unwrap_or_default(),
        launch_log_path()
            .map(|path| path.display().to_string())
            .unwrap_or_default()
    )
}

#[cfg(target_os = "macos")]
fn macos_install(runtime_dir: &Path) -> Result<ServiceStatus> {
    let unit_path = macos_unit_path()
        .ok_or_else(|| CliError::usage("cannot locate HOME to write the LaunchAgent"))?;
    if unit_path.exists() {
        // 已经有服务在管这个 runtime 了 —— 可能来自 macOS App。
        // 不覆盖它：一个机器只有一个 Runtime。
        let loaded = launchctl("list").is_ok_and(|output| output.contains(LAUNCH_AGENT_LABEL));
        if loaded {
            return Ok(macos_status(runtime_dir));
        }
    }
    let program = current_exe()?;
    let plist = macos_plist(&program, runtime_dir);
    if let Some(parent) = unit_path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| {
            CliError::usage(format!("cannot create {}: {error}", parent.display()))
        })?;
    }
    std::fs::write(&unit_path, plist).map_err(|error| {
        CliError::usage(format!("cannot write {}: {error}", unit_path.display()))
    })?;

    // 已经在跑就别重复 bootstrap。
    if !launchctl("list").is_ok_and(|output| output.contains(LAUNCH_AGENT_LABEL)) {
        launchctl(&format!(
            "bootstrap gui/{} {}",
            current_uid(),
            unit_path.display()
        ))
        .map_err(|error| {
            CliError::usage(format!(
                "cannot load the LaunchAgent ({}). 你可以手动运行 `else-bot --dir {} serve`。",
                error,
                runtime_dir.display()
            ))
        })?;
    }
    Ok(macos_status(runtime_dir))
}

#[cfg(target_os = "macos")]
fn macos_uninstall(runtime_dir: &Path) -> Result<ServiceStatus> {
    let unit_path = macos_unit_path()
        .ok_or_else(|| CliError::usage("cannot locate HOME to remove the LaunchAgent"))?;
    if unit_path.exists() {
        let _ = launchctl(&format!(
            "bootout gui/{}/{LAUNCH_AGENT_LABEL}",
            current_uid()
        ));
        std::fs::remove_file(&unit_path).map_err(|error| {
            CliError::usage(format!("cannot remove {}: {error}", unit_path.display()))
        })?;
    }
    Ok(macos_status(runtime_dir))
}

#[cfg(target_os = "linux")]
fn linux_status(runtime_dir: &Path) -> ServiceStatus {
    let unit_path = linux_unit_path();
    let installed = unit_path.as_ref().is_some_and(|path| path.exists());
    let active = systemctl(&["--user", "is-active", SYSTEMD_UNIT])
        .is_ok_and(|output| output.trim() == "active");
    let unit_runtime_dir = unit_path
        .as_ref()
        .and_then(|path| std::fs::read_to_string(path).ok())
        .and_then(|body| {
            body.lines()
                .find_map(|line| line.strip_prefix("ExecStart="))
                .and_then(|command| {
                    command
                        .split_whitespace()
                        .collect::<Vec<_>>()
                        .windows(2)
                        .find(|pair| pair[0] == "--dir")
                        .map(|pair| PathBuf::from(pair[1]))
                })
        });
    let detail = if active {
        format!("systemd user unit {SYSTEMD_UNIT} 正在运行")
    } else if installed {
        "systemd user unit 已安装但没有运行；运行 `else-bot daemon install` 启动它".into()
    } else {
        "没有注册后台服务；运行 `else-bot daemon install`".into()
    };
    ServiceStatus {
        platform: "linux",
        unit_path,
        installed,
        loaded: active,
        running: active,
        unit_runtime_dir,
        detail,
    }
    .with_runtime(runtime_dir)
}

/// Linux systemd user unit 的内容。
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn linux_unit(program: &Path, runtime_dir: &Path) -> String {
    format!(
        "[Unit]\n\
         Description=else Thread Runtime\n\
         After=default.target\n\
         \n\
         [Service]\n\
         Type=simple\n\
         ExecStart={} --dir {} serve\n\
         Restart=on-failure\n\
         RestartSec=5\n\
         \n\
         [Install]\n\
         WantedBy=default.target\n",
        program.display(),
        runtime_dir.display()
    )
}

#[cfg(target_os = "linux")]
fn linux_install(runtime_dir: &Path) -> Result<ServiceStatus> {
    let unit_path = linux_unit_path()
        .ok_or_else(|| CliError::usage("cannot locate the systemd user unit directory"))?;
    let program = current_exe()?;
    let unit = linux_unit(&program, runtime_dir);
    if let Some(parent) = unit_path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| {
            CliError::usage(format!("cannot create {}: {error}", parent.display()))
        })?;
    }
    std::fs::write(&unit_path, unit).map_err(|error| {
        CliError::usage(format!("cannot write {}: {error}", unit_path.display()))
    })?;
    systemctl(&["--user", "daemon-reload"])
        .map_err(|error| CliError::usage(format!("cannot reload systemd user units: {error}")))?;
    systemctl(&["--user", "enable", "--now", SYSTEMD_UNIT]).map_err(|error| {
        CliError::usage(format!(
            "cannot start {SYSTEMD_UNIT} ({error}). \
             登出后还想继续运行的话，执行 `loginctl enable-linger $USER`。"
        ))
    })?;
    Ok(linux_status(runtime_dir))
}

#[cfg(target_os = "linux")]
fn linux_uninstall(runtime_dir: &Path) -> Result<ServiceStatus> {
    let unit_path = linux_unit_path()
        .ok_or_else(|| CliError::usage("cannot locate the systemd user unit directory"))?;
    if unit_path.exists() {
        let _ = systemctl(&["--user", "disable", "--now", SYSTEMD_UNIT]);
        std::fs::remove_file(&unit_path).map_err(|error| {
            CliError::usage(format!("cannot remove {}: {error}", unit_path.display()))
        })?;
        let _ = systemctl(&["--user", "daemon-reload"]);
    }
    Ok(linux_status(runtime_dir))
}

/// 状态里带上 runtime 目录。
impl ServiceStatus {
    /// 描述里带上"这个状态是对哪个 runtime 目录说的"。
    /// 注意：目录来自调用方（当前会话），unit 文件真正指向哪里见
    /// `unit_runtime_dir`，两者不一致由 doctor 报错。
    fn with_runtime(mut self, runtime_dir: &Path) -> Self {
        self.detail = format!("{}（当前会话：{}）", self.detail, runtime_dir.display());
        self
    }
}

/// 调用 launchctl 并返回输出。
#[cfg(target_os = "macos")]
fn launchctl(args: &str) -> std::result::Result<String, String> {
    let output = std::process::Command::new("launchctl")
        .args(args.split_whitespace())
        .output()
        .map_err(|error| error.to_string())?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

/// 调用 systemctl 并返回输出。
#[cfg(target_os = "linux")]
fn systemctl(args: &[&str]) -> std::result::Result<String, String> {
    let output = std::process::Command::new("systemctl")
        .args(args)
        .output()
        .map_err(|error| error.to_string())?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

/// 当前用户的 uid。
#[cfg(target_os = "macos")]
fn current_uid() -> u32 {
    // SAFETY: getuid 没有参数也没有内存影响。
    unsafe { libc::getuid() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(target_os = "macos")]
    fn macos_launch_agent_points_at_the_runtime_directory() {
        let plist = macos_plist(
            Path::new("/usr/local/bin/else"),
            Path::new("/Users/someone/Library/Application Support/else/runtime"),
        );
        assert!(plist.contains("<string>com.else.desktop.runtime</string>"));
        assert!(plist.contains("<string>/usr/local/bin/else</string>"));
        assert!(plist.contains("<string>serve</string>"));
        assert!(plist.contains("Application Support/else/runtime"));
        // 登录后自动起来，并且掉了自动拉起。
        assert!(plist.contains("<key>RunAtLoad</key>"));
        assert!(plist.contains("<key>KeepAlive</key>"));
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn linux_unit_points_at_the_runtime_directory() {
        let unit = linux_unit(
            Path::new("/usr/local/bin/else"),
            Path::new("/home/someone/.local/share/else/runtime"),
        );
        assert!(unit.contains(
            "ExecStart=/usr/local/bin/else --dir /home/someone/.local/share/else/runtime serve"
        ));
        assert!(unit.contains("WantedBy=default.target"));
    }
}
