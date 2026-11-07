# 平台与架构

这份文档回答一件事：**else 在哪种机器上跑到哪一步，以及哪些差异是能力差异，
不是 bug。**

代码先按这里改；平台相关的改动必须同时更新本文的表格。

## 1. 声称支持的范围

| 系统 | 架构 | 状态下 |
| --- | --- | --- |
| macOS | aarch64 | 构建 + 测试 |
| macOS | x86_64 | 只构建，从不测试 |
| Linux (GNU) | x86_64 | 构建 + 测试 |
| Linux (GNU) | aarch64 | 只构建，从不测试 |
| 其它 | — | 显式报错，不降级 |

"只构建"的意思是：发布流水线会产出制品，但没有任何测试跑在那个架构上。
一个只在 x86_64 Linux 上成立的行为，在 aarch64 Linux 上是没人验证过的。

## 2. 每个平台各由什么实现

| 能力 | macOS | Linux | 其它 |
| --- | --- | --- | --- |
| 沙箱 | Seatbelt (`/usr/bin/sandbox-exec`) | bubblewrap (`/usr/bin/bwrap`) | 报错 `UnsupportedPlatform` |
| 出网白名单 | 支持：harness 出网指向本地代理 | **不支持**（见 §3） | — |
| 凭据 | Keychain（默认，失败关闭不降级） | 私有文件 | 私有文件 |
| 常驻服务 | LaunchAgent `com.else.desktop.runtime` | systemd user unit `else.service` | 无（只能 `serve` 前台） |
| runtime 目录 | `~/Library/Application Support/else/runtime` | `$XDG_DATA_HOME/else/runtime`，否则 `~/.local/share/else/runtime` | 同 Linux |
| 终端通知 | 按 `TERM_PROGRAM` 判断（iTerm2 / Ghostty / WezTerm / kitty） | 同左 | 不发 |
| 进程存活判断 | `kill(pid, 0)` | 同左 | 恒为真（即无法判断，仅占位） |

凭据后端可以用 `ELSE_CREDENTIAL_BACKEND=file|keychain` 覆盖；`keychain` 只在
macOS 上存在，在别的系统上显式报错而不是悄悄退回文件。

沙箱可执行文件找不到时会报 `BackendUnavailable`，**拒绝无沙箱执行**，不会
在没有边界的情况下跑用户的工具。

## 3. 已知的能力差异

### 3.1 Linux 上带网络的 opaque harness 跑不起来

`NetworkPolicy::Proxy`（"所有出网都经过本地代理 + 白名单"）只有 Seatbelt 能
兑现（`Backend::supports_local_proxy`）。bubblewrap 后端下构造沙箱会返回
`ProxyUnsupported`，也就是**失败关闭**，而不是放开网络。

后果：`claude-code`、`codex` 这类自己出网的 harness，在 Linux 上会被拒绝执行；
Linux 实际能跑的是不联网的 harness。

这不是疏漏，是有意的取舍：宁可不跑，也不要在没有白名单的情况下让 agent 随便
出网。真正的解法是给 bubblewrap 一个等价方案（网络命名空间 + slirp 之类的本
地代理），那是独立的一项工作。

`else-bot doctor` 会把这台机器的沙箱后端和它能不能做白名单代理直接印出来，
不需要读源码才知道。

### 3.2 架构只被构建、不被测试

aarch64 Linux 与 x86_64 macOS 在发布矩阵里，但不在 CI 的测试矩阵里。任何
架构相关的问题（字节序、对齐、`libc` 结构体布局、SIMD）都不会被自动发现。

## 4. 编译期陷阱：本地绿不等于合并绿

`#[cfg(target_os = ...)]` 的代码**在本机根本不参与编译**。

**0.1.3 的事故**：给 `ServiceStatus` 加了 `unit_program` 字段，`macos_status`
填了，`linux_status` 没填。macOS 上 `./scripts/dev.sh check` 全绿、238 个测试
全过；推上去之后 Linux CI 直接编译失败（clippy exit 101）。

### 规则

1. **平台专属代码只能由那个平台的构建来证明。** 本地的绿是一件平台上的绿。
2. **平台分支不写完整结构体字面量。** 所有 `ServiceStatus` 从
   `ServiceStatus::empty(platform)` 铺底、再用 `..` 覆盖字段 —— 这样新增字段
   不可能只在一个平台漏填。测试
   `every_service_status_is_built_from_the_shared_default` 把这条规则钉死。
3. **能力差异要么实现，要么说出来。** 不许让"这台机器做不到"只体现在一段
   读不到的代码里：`doctor`、能力声明（`else-bot harnesses`）、文档三处之一
   必须讲清楚。

## 5. 本地怎么验别的平台

`cargo check --target x86_64-unknown-linux-gnu` 目前**跑不通**：

```
error: failed to run custom build command for `ring v0.17.14`
```

依赖链是 `else-gateway → genai → reqwest → rustls → ring`，`ring` 的 build
script 需要目标平台的 C 工具链。也就是说，本机没有 Linux 交叉工具链时，跨平台
类型检查这条路是断的。

所以：

- Linux 的门禁是 **CI**（`ubuntu-latest`），本地只能靠 §4 的结构性规则兜底。
- 有 Docker 时可以整机验：`docker run --rm -v "$PWD:/w" -w /w rust:1 cargo check --workspace`
  （首次要拉镜像，比 CI 慢，但比等一轮 CI 快）。
- 真正要改平台行为时，改完看 CI 的 `check (ubuntu-latest)` 那一格，别只看本机。

## 6. 发布矩阵

`release-cli.yml` 为四个 target 各产一份制品：

```
x86_64-unknown-linux-gnu
aarch64-unknown-linux-gnu
aarch64-apple-darwin
x86_64-apple-darwin
```

CI 的测试矩阵只有 `ubuntu-latest`（x86_64）和 `macos-latest`（arm64），外加
`apple-client` 在 `macos-15` 上跑 `swift test` 与打包。

Homebrew formula 里有四个 URL，checksum 由 `scripts/update-homebrew-formula.py`
在发布时填 —— **不要手改**，手工填的 checksum 会让 formula 装不上。

## 7. 加平台相关代码的检查清单

- [ ] 结构体从共享的构造函数铺底（`..T::empty(..)`），不抄一份完整字面量。
- [ ] 新分支在 CI 矩阵里有对应的 job；没有就先加 job，再加代码。
- [ ] 能力差异写进 `doctor` 或能力声明，而不是只写在代码注释里。
- [ ] 更新本文 §1、§2 的表格。
- [ ] 如果差异会影响产品承诺（比如"能不能联网"），在 README 或界面里也说出来。
