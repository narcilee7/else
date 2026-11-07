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
    /// 服务定义实际启动的程序。
    ///
    /// 端口上跑的是不是"你以为的那个程序"，只能从这里看出来 ——
    /// 服务注册在一个旧安装路径上时，一切都像是正常的。
    pub unit_program: Option<PathBuf>,
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

/// 一次注册实际做了什么。
///
/// 注册是**对账**，不是"文件不在就写一份"：服务定义指向别的程序或别的
/// runtime 目录时，它必须被改回来，否则端口上跑的永远是上一次装的那个。
#[derive(Debug, Clone, serde::Serialize)]
pub struct InstallReport {
    pub status: ServiceStatus,
    /// 这次实际改动的地方。空表示本来就已经是对的。
    pub actions: Vec<String>,
}

/// 注册后台服务。已经装过也会对账：指向不对就改回当前这个二进制。
pub fn install(runtime_dir: &Path) -> Result<InstallReport> {
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

/// 服务定义与实际不符时的说明。对账要说出**改了什么**，否则用户没法判断
/// 它到底修没修好。
#[cfg(target_os = "macos")]
fn describe_change(existing: Option<&str>, program: &Path, runtime_dir: &Path) -> Vec<String> {
    let Some(existing) = existing else {
        return vec![format!(
            "wrote a new service definition for {}",
            program.display()
        )];
    };
    let old_program = plist_first_program_argument(existing);
    let old_dir = plist_argument(existing, "--dir");
    let mut changes = Vec::new();
    match old_program {
        Some(old) if old != program => changes.push(format!(
            "the service started {}, it now starts {}",
            old.display(),
            program.display()
        )),
        None => changes.push(format!(
            "the service did not name a program, it now starts {}",
            program.display()
        )),
        _ => {}
    }
    match old_dir {
        Some(old) if old != runtime_dir => changes.push(format!(
            "the service served {}, it now serves {}",
            old.display(),
            runtime_dir.display()
        )),
        None => changes.push(format!(
            "the service did not name a runtime directory, it now serves {}",
            runtime_dir.display()
        )),
        _ => {}
    }
    if changes.is_empty() {
        changes.push("the definition format changed; it was regenerated".to_string());
    }
    changes
}

/// 对账之后该做什么。
///
/// 这是一个**纯判断**：真正动手会 bootout 用户的 LaunchAgent，所以先把
/// 决定和动作分开，决定可以随便测。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Repair {
    /// 已经一致。
    Nothing,
    /// 重装一次就能对上。
    Reinstall(Vec<String>),
    /// 问题不在服务定义上，重装解决不了 —— 说清楚卡在哪。
    CannotFix(String),
}

/// 判断要不要重装，以及为什么。
///
/// `foreign_holder` 是端口上那个"不是我们启动的进程"的描述。定义本身指向
/// 别处时优先重装：那种情况下占着端口的往往正是上一次装的那个进程，
/// bootout 会把它带走。定义已经对了却还有别人占端口，就不是重装能解决的。
pub fn plan_repair(
    status: &ServiceStatus,
    current_program: Option<&Path>,
    runtime_dir: &Path,
    foreign_holder: Option<&str>,
) -> Repair {
    let mut reasons = Vec::new();
    if let (Some(installed), Some(current)) = (status.unit_program.as_deref(), current_program) {
        if installed != current {
            reasons.push(format!(
                "the service starts {}, this command is {}",
                installed.display(),
                current.display()
            ));
        }
    }
    if let Some(unit_dir) = status.unit_runtime_dir.as_deref() {
        if unit_dir != runtime_dir {
            reasons.push(format!(
                "the service serves {}, this command manages {}",
                unit_dir.display(),
                runtime_dir.display()
            ));
        }
    }
    if !reasons.is_empty() {
        return Repair::Reinstall(reasons);
    }
    if !status.installed {
        return Repair::Reinstall(vec!["no background service is registered".into()]);
    }
    if !status.running {
        return Repair::Reinstall(vec!["the service is registered but not running".into()]);
    }
    match foreign_holder {
        Some(holder) => Repair::CannotFix(format!(
            "{holder} 占着端口，而服务定义本身是对的 —— 重装不会让它让开。\
             先停掉那个进程，再运行 `else-bot doctor`。"
        )),
        None => Repair::Nothing,
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

impl ServiceStatus {
    /// 空状态。
    ///
    /// 每个平台都从这个起点填，新增字段时就不会出现"只在一个平台上漏填、
    /// 而那段代码在本机根本不编译"的情况 —— 那种问题只会在 Linux CI 上
    /// 冒出来。
    fn empty(platform: &'static str) -> Self {
        Self {
            platform,
            unit_path: None,
            installed: false,
            loaded: false,
            running: false,
            unit_runtime_dir: None,
            unit_program: None,
            detail: String::new(),
        }
    }
}

/// 没有后台服务支持的平台。
#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn unsupported_status() -> ServiceStatus {
    ServiceStatus {
        detail: "这个平台还没有后台服务支持；可以用 `else-bot serve` 前台运行。".into(),
        ..ServiceStatus::empty("unsupported")
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
    let unit_body = unit_path
        .as_ref()
        .and_then(|path| std::fs::read_to_string(path).ok());
    let unit_runtime_dir = unit_body
        .as_deref()
        .and_then(|body| plist_argument(body, "--dir"));
    let unit_program = unit_body.as_deref().and_then(plist_first_program_argument);
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
        unit_path,
        installed,
        loaded,
        running,
        unit_runtime_dir,
        unit_program,
        detail,
        ..ServiceStatus::empty("macos")
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

/// plist `ProgramArguments` 的第一个元素就是真正被启动的程序。
#[cfg(target_os = "macos")]
fn plist_first_program_argument(body: &str) -> Option<PathBuf> {
    let start = body.find("<key>ProgramArguments</key>")?;
    let rest = &body[start..];
    let array = rest.find("<array>")?;
    let value = rest[array..].split("<string>").nth(1)?;
    let program = value.split("</string>").next()?.trim();
    (!program.is_empty() && !program.starts_with('-')).then(|| PathBuf::from(program))
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
fn macos_install(runtime_dir: &Path) -> Result<InstallReport> {
    let unit_path = macos_unit_path()
        .ok_or_else(|| CliError::usage("cannot locate HOME to write the LaunchAgent"))?;
    let program = current_exe()?;
    let desired = macos_plist(&program, runtime_dir);
    let existing = std::fs::read_to_string(&unit_path).ok();
    let loaded = launchctl("list").is_ok_and(|output| output.contains(LAUNCH_AGENT_LABEL));
    let mut actions = Vec::new();

    // 定义已经一致、而且进程真的在跑：什么都不用做。
    let matches = existing.as_deref() == Some(desired.as_str());
    if matches && loaded {
        return Ok(InstallReport {
            status: macos_status(runtime_dir),
            actions,
        });
    }

    if !matches {
        actions.extend(describe_change(existing.as_deref(), &program, runtime_dir));
        if let Some(parent) = unit_path.parent() {
            std::fs::create_dir_all(parent).map_err(|error| {
                CliError::usage(format!("cannot create {}: {error}", parent.display()))
            })?;
        }
        std::fs::write(&unit_path, &desired).map_err(|error| {
            CliError::usage(format!("cannot write {}: {error}", unit_path.display()))
        })?;
    }

    // 已经加载过就要先卸掉再装：只改文件不会让新程序接管这个 label。
    if loaded {
        launchctl(&format!(
            "bootout gui/{}/{LAUNCH_AGENT_LABEL}",
            current_uid()
        ))
        .map_err(|error| {
            CliError::usage(format!(
                "cannot stop the running LaunchAgent ({error}). \
                     手动执行 `launchctl bootout gui/{}/{LAUNCH_AGENT_LABEL}` 后再试。",
                current_uid()
            ))
        })?;
        actions.push("stopped the previous instance".to_string());
    }

    launchctl(&format!(
        "bootstrap gui/{} {}",
        current_uid(),
        unit_path.display()
    ))
    .map_err(|error| {
        CliError::usage(format!(
            "cannot load the LaunchAgent ({error}). 你可以手动运行 `else-bot --dir {} serve`。",
            runtime_dir.display()
        ))
    })?;
    actions.push(format!(
        "started the background runtime from {}",
        program.display()
    ));

    Ok(InstallReport {
        status: macos_status(runtime_dir),
        actions,
    })
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
    // ExecStart 里既有程序也有 --dir：两样都要读出来，否则"服务指向别处"
    // 这件事在 Linux 上根本看不见。
    let exec: Vec<String> = unit_path
        .as_ref()
        .and_then(|path| std::fs::read_to_string(path).ok())
        .and_then(|body| {
            body.lines()
                .find_map(|line| line.strip_prefix("ExecStart="))
                .map(|command| {
                    command
                        .split_whitespace()
                        .map(str::to_string)
                        .collect::<Vec<_>>()
                })
        })
        .unwrap_or_default();
    let unit_program = exec.first().map(PathBuf::from);
    let unit_runtime_dir = exec
        .windows(2)
        .find(|pair| pair[0] == "--dir")
        .map(|pair| PathBuf::from(&pair[1]));
    let detail = if active {
        format!("systemd user unit {SYSTEMD_UNIT} 正在运行")
    } else if installed {
        "systemd user unit 已安装但没有运行；运行 `else-bot daemon install` 启动它".into()
    } else {
        "没有注册后台服务；运行 `else-bot daemon install`".into()
    };
    ServiceStatus {
        unit_path,
        installed,
        loaded: active,
        running: active,
        unit_runtime_dir,
        unit_program,
        detail,
        ..ServiceStatus::empty("linux")
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
fn linux_install(runtime_dir: &Path) -> Result<InstallReport> {
    let unit_path = linux_unit_path()
        .ok_or_else(|| CliError::usage("cannot locate the systemd user unit directory"))?;
    let program = current_exe()?;
    let unit = linux_unit(&program, runtime_dir);
    let existing = std::fs::read_to_string(&unit_path).ok();
    let mut actions = Vec::new();
    if existing.as_deref() != Some(unit.as_str()) {
        actions.push(format!(
            "wrote a service definition for {} serving {}",
            program.display(),
            runtime_dir.display()
        ));
        if let Some(parent) = unit_path.parent() {
            std::fs::create_dir_all(parent).map_err(|error| {
                CliError::usage(format!("cannot create {}: {error}", parent.display()))
            })?;
        }
        std::fs::write(&unit_path, unit).map_err(|error| {
            CliError::usage(format!("cannot write {}: {error}", unit_path.display()))
        })?;
    }
    systemctl(&["--user", "daemon-reload"])
        .map_err(|error| CliError::usage(format!("cannot reload systemd user units: {error}")))?;
    systemctl(&["--user", "enable", "--now", SYSTEMD_UNIT]).map_err(|error| {
        CliError::usage(format!(
            "cannot start {SYSTEMD_UNIT} ({error}). \
             登出后还想继续运行的话，执行 `loginctl enable-linger $USER`。"
        ))
    })?;
    actions.push(format!(
        "started the background runtime from {}",
        program.display()
    ));
    Ok(InstallReport {
        status: linux_status(runtime_dir),
        actions,
    })
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

    fn status(program: Option<&str>, dir: Option<&str>, running: bool) -> ServiceStatus {
        ServiceStatus {
            unit_path: Some(PathBuf::from("/tmp/test.plist")),
            installed: true,
            loaded: running,
            running,
            unit_runtime_dir: dir.map(PathBuf::from),
            unit_program: program.map(PathBuf::from),
            ..ServiceStatus::empty("macos")
        }
    }

    /// 端口上跑的是上一次装的那个二进制时，重装是解药。
    #[test]
    fn a_service_pointing_at_another_binary_is_reinstalled() {
        let current = PathBuf::from("/opt/else/bin/else-bot");
        let repair = plan_repair(
            &status(Some("/Users/x/.local/bin/else"), Some("/data/else"), true),
            Some(&current),
            Path::new("/data/else"),
            Some("pid 42（它比这条命令旧）"),
        );
        let Repair::Reinstall(reasons) = repair else {
            panic!("expected a reinstall, got {repair:?}");
        };
        let joined = reasons.join(" | ");
        assert!(joined.contains("/Users/x/.local/bin/else"), "{joined}");
        assert!(joined.contains("/opt/else/bin/else-bot"), "{joined}");
    }

    #[test]
    fn a_service_serving_another_runtime_directory_is_reinstalled() {
        let current = PathBuf::from("/opt/else/bin/else-bot");
        let repair = plan_repair(
            &status(Some("/opt/else/bin/else-bot"), Some("/other"), true),
            Some(&current),
            Path::new("/data/else"),
            None,
        );
        assert!(matches!(repair, Repair::Reinstall(_)), "{repair:?}");
    }

    #[test]
    fn a_matching_definition_that_is_running_is_left_alone() {
        let current = PathBuf::from("/opt/else/bin/else-bot");
        let repair = plan_repair(
            &status(Some("/opt/else/bin/else-bot"), Some("/data/else"), true),
            Some(&current),
            Path::new("/data/else"),
            None,
        );
        assert_eq!(repair, Repair::Nothing);
    }

    /// 定义是对的、端口却还被别人占着：重装只会白折腾，也不替用户杀进程。
    #[test]
    fn a_foreign_process_holding_the_port_is_not_killed_for_you() {
        let current = PathBuf::from("/opt/else/bin/else-bot");
        let repair = plan_repair(
            &status(Some("/opt/else/bin/else-bot"), Some("/data/else"), true),
            Some(&current),
            Path::new("/data/else"),
            Some("pid 777（它不报自己替哪个目录服务）"),
        );
        let Repair::CannotFix(reason) = repair else {
            panic!("expected a refusal, got {repair:?}");
        };
        assert!(reason.contains("pid 777"), "{reason}");
    }

    #[test]
    fn a_registered_service_that_is_not_running_is_started() {
        let current = PathBuf::from("/opt/else/bin/else-bot");
        let repair = plan_repair(
            &status(Some("/opt/else/bin/else-bot"), Some("/data/else"), false),
            Some(&current),
            Path::new("/data/else"),
            None,
        );
        assert!(matches!(repair, Repair::Reinstall(_)), "{repair:?}");
    }

    /// 每个平台都必须从 `ServiceStatus::empty` 铺底再覆盖字段。
    ///
    /// 这就是 0.1.3 在 Linux CI 上编译失败的原因：`linux_status` 少填了一个
    /// 新字段，而那段代码在 macOS 上根本不参与编译，本地怎么跑都是绿的。
    /// 这个测试把"手写完整字面量"这件事本身禁掉。
    #[test]
    fn every_service_status_is_built_from_the_shared_default() {
        let source = include_str!("service.rs");
        let lines: Vec<&str> = source.lines().collect();
        for (index, line) in lines.iter().enumerate() {
            let trimmed = line.trim();
            // 只认结构体字面量本身；`-> ServiceStatus {` 是返回类型，不是构造。
            if !trimmed.starts_with("ServiceStatus {") {
                continue;
            }
            // 找到这个字面量的结尾，看中间有没有铺底。
            let mut padded = false;
            for look in lines.iter().skip(index).take(24) {
                if look.contains("..ServiceStatus::empty(") {
                    padded = true;
                    break;
                }
                if look.trim() == "}" {
                    break;
                }
            }
            assert!(
                padded,
                "line {}: build ServiceStatus from `ServiceStatus::empty(..)` and override fields",
                index + 1
            );
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn the_plist_names_the_program_and_the_runtime_directory() {
        let body = macos_plist(
            Path::new("/opt/else/bin/else-bot"),
            Path::new("/Users/x/Library/Application Support/else/runtime"),
        );
        assert_eq!(
            plist_first_program_argument(&body).as_deref(),
            Some(Path::new("/opt/else/bin/else-bot"))
        );
        assert_eq!(
            plist_argument(&body, "--dir").as_deref(),
            Some(Path::new(
                "/Users/x/Library/Application Support/else/runtime"
            ))
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn a_changed_definition_explains_what_changed() {
        let old = macos_plist(
            Path::new("/Users/x/.local/bin/else"),
            Path::new("/old/runtime"),
        );
        let changes = describe_change(
            Some(&old),
            Path::new("/opt/else/bin/else-bot"),
            Path::new("/new/runtime"),
        );
        let joined = changes.join(" | ");
        assert!(joined.contains("/Users/x/.local/bin/else"), "{joined}");
        assert!(joined.contains("/opt/else/bin/else-bot"), "{joined}");
        assert!(joined.contains("/old/runtime"), "{joined}");
        assert!(joined.contains("/new/runtime"), "{joined}");
    }
}
