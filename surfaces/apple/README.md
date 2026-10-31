# else Apple client

macOS 原生壳，使用 SwiftUI 与 Swift Package Manager。目标不是再做一个聊天窗口，而是把 Runtime 投影收敛成三个主表面与常驻体验：

- 今天：可解释 Priority、来源证据、分组 Personal Brief、外部事项、进行中 Intent 与 Decision 聚合
- 世界：Project / Contact / Customer / Thread / Milestone 管理，支持 Rename / Merge / Split
- 连接：Adapter 发现、连接器注册、凭据安全录入、Health 与手动同步
- 待决策：只有真正需要本人的 Decision，按 Batch/Defer Window 聚合，可直接完成常用选项或进入完整 Intent
- 结果：已收敛的 Outcome、证据与资源账；详情含 ConnectorAction 执行、核验、补偿与最终副作用结论
- 动作创建：Intent 详情可选 Connector / 写 Capability 和目标；Capability Manifest 声明字段 Schema，新 Provider 可自动使用通用表单，Calendar、Linear、Notion 使用专用结构化表单；执行前可读取 Provider 当前值并查看字段级 Diff，高级 JSON 保留为 Escape Hatch
- 常驻体验：内嵌 Runtime daemon、独立通知 Helper、SMAppService LaunchAgent、15/30/60 秒后台刷新、去重系统通知、Dock Badge、菜单栏 Quick Access 与设置页
- macOS 原生入口：`else://` Deep Link、Services“用 else 处理”与 App Intents/Shortcuts
- 数据导出：Settings 可导出 Intent、Event Ledger、工件内容与血缘；凭据本体和私有 Harness 状态不会进入导出
- 数据删除：Intent 详情支持二次确认后的永久删除，会删除事件、关联凭据和 Scope Workspace
- 数据 Retention：Settings 可选择 7 / 30 / 90 天后自动清理终态 Intent；默认关闭且开启需二次确认
- 诊断报告：Settings 可导出数据库完整性、Connector Health、Storage、Retention 与运行版本的脱敏 JSON
- Connector 修复：不健康连接可重建 Cursor 并重新同步，退避状态会先清零
- Connector 认证：支持重新录入凭据、替换 Secret 引用与二次确认撤销
- Lease 修复：支持清理已结束 Run / 终态 Scope 的孤儿 Lease，活跃 Lease 不受影响
- 发布：Developer ID hardened runtime 签名、notarytool 公证、staple 与 SHA-256 打包脚本

视觉、Motion 与组件约束见 [DESIGN.md](../../DESIGN.md)。

## 结构

```text
surfaces/apple/
├── Sources/ElseClientCore/   # 协议 DTO、RuntimeClient、HTTP 传输、Demo 实现
├── Sources/ElseMac/          # SwiftUI 应用、状态容器与本地配置
├── Sources/ElseNotifier/     # 主 App 退出后的 Decision 通知 Helper
├── Resources/LaunchAgents/   # 内嵌 Runtime / 通知 LaunchAgent
├── Tests/ElseClientCoreTests/
├── Resources/Info.plist
└── scripts/build-app.sh
```

`ElseClientCore` 不依赖 SwiftUI，后续 iOS、菜单栏、Widget 与 App Intents 可复用。客户端不直接链接 Rust `Runtime`，只通过版本化 Runtime API 工作。

Deep Link 支持：

```text
else://today
else://world
else://connectors
else://decisions
else://results
else://new-intent
else://intent/{intent_id}
```

在支持 Services 的 App 中选中文本后，可从“服务”菜单选择“用 else 处理”提交 Intent。App Intents 提供“提交 else Intent”和“查看 else 今天”；Shortcuts 自动发现需要完整 Xcode 构建生成 App Intents Metadata，Command Line Tools 构建只验证代码编译。

## 本地运行

```bash
cd surfaces/apple
swift test

# 不需要 daemon 的 UI 开发模式
swift run ElseMac --demo

# 生成可启动的 .app bundle
./scripts/build-app.sh
open .build/Else.app --args --demo

# 生成包含 debug Runtime Helper 的 App
./scripts/build-app.sh debug
open .build/Else.app

# 生成包含 release Runtime Helper 的 App
./scripts/build-app.sh release

# Developer ID / notarization 发布
CODESIGN_IDENTITY="Developer ID Application: ..." \
APPLE_ID="..." APPLE_APP_PASSWORD="..." APPLE_TEAM_ID="..." \
./scripts/package-release.sh
```

正式模式默认连接 `http://127.0.0.1:4180`，可通过 `ELSE_RUNTIME_URL` 覆盖。Bearer Token 的解析顺序为：

1. `ELSE_RUNTIME_TOKEN`。
2. loopback Runtime 下，`~/Library/Application Support/else/runtime/daemon-token`。
3. 都没有时保持未连接。

可通过 `ELSE_RUNTIME_DIR` 覆盖 Runtime 数据目录。只有当 Runtime URL 是 loopback 时才会自动读取本地 daemon Token，避免把本机 Token 发给远程 Runtime。`ELSE_DEMO=1` 或 `--demo` 只用于开发与视觉验收。

App bundle 包含：

```text
Else.app/Contents/
├── MacOS/
│   ├── ElseMac
│   └── ElseNotifier
├── Helpers/else-daemon
└── Library/LaunchAgents/
    ├── com.else.desktop.runtime.plist
    └── com.else.desktop.notifier.plist
```

正式签名构建会先签嵌套 daemon 与通知 Helper，再签整 App。Settings 可查看 Runtime / 通知 Helper 与 Token 状态，执行启动、停止、重启、重连和打开数据目录。ad-hoc 构建可以运行 App、内嵌 daemon 与通知 Helper，但 macOS 可能让 `SMAppService` 返回 `notFound`；完整的后台服务注册需要使用 Developer ID 构建和执行。

`ElseNotifier` 位于主 App bundle 内，因此复用 `com.else.desktop` 的通知授权。它只在主 App 未运行时投递新 Decision，状态写入 Runtime 数据目录下的 0600 `notifier-state.json`。点击通知会以 `--open-intent <id>` 拉起主 App。

开发时可单独检查通知 Helper：

```bash
.build/Else.app/Contents/MacOS/ElseNotifier --check
.build/Else.app/Contents/MacOS/ElseNotifier --once
```

## Runtime API v1

以下 HTTP 契约由 `else serve` / `else-daemon` 实现。除健康检查外均要求 `Authorization: Bearer <token>`；POST 要求 `Idempotency-Key`，run/Decision/Abort 还要求 `If-Match: <intent_revision>`：

| Method | Path | Response |
| --- | --- | --- |
| `GET` | `/v1/intents` | `[IntentSnapshot]` |
| `GET` | `/v1/intents/{intent_id}` | `IntentSnapshot` |
| `POST` | `/v1/intents` | `IntentSnapshot` |
| `POST` | `/v1/intents/{intent_id}/run` | `IntentSnapshot` |
| `POST` | `/v1/intents/{intent_id}/decisions/{decision_id}/resolve` | `IntentSnapshot` |
| `POST` | `/v1/intents/{intent_id}/abort` | `IntentSnapshot` |
| `GET` | `/v1/world/brief` | `PersonalBrief` |
| `POST` | `/v1/intents/{intent_id}/world/resolve` | `WorldItem` |
| `GET` | `/v1/operations/{operation_id}` | `Operation` |
| `GET` | `/v1/events?cursor=42` | `[Event]` |
| `GET/POST` | `/v1/intents/{intent_id}/capabilities` | `[Capability]` / `Capability` |
| `GET/POST` | `/v1/intents/{intent_id}/bindings` | `[AgentBinding]` / `AgentBinding` |
| `POST` | `/v1/intents/{intent_id}/bindings/{binding_id}/revoke` | `{"ok": true}` |
| `POST` | `/v1/intents/{intent_id}/bindings/{binding_id}/dispatch` | `IntentSnapshot` |
| `POST` | `/v1/intents/{intent_id}/bindings/{binding_id}/state` | `{"ok": true}` |
| `GET` | `/v1/intents/{intent_id}/bindings/{binding_id}/events?cursor=42` | `[BindingEvent]` |
| `GET` | `/v1/intents/{intent_id}/bindings/{binding_id}/events/stream` | SSE `text/event-stream` |
| `POST` | `/v1/binding-events` | Agent callback using binding session Bearer |

Connector Action 的当前值 Preview 使用：

```text
POST /v1/intents/{intent_id}/connectors/{connector_id}/actions/preview
```

请求体包含 `capability`、`target` 与 `input`；响应返回 Provider 当前可写字段和拟提交字段，不创建 Action，也不产生外部写副作用。

Runtime 数据导出使用：

```text
GET /v1/export
```

响应为版本化 JSON，包含 Intent、完整事件流、工件内容与 Credential 元数据引用。daemon Token、凭据本体、私有 Harness 目录和越界工件路径不会写入导出。

Intent 永久删除使用：

```text
DELETE /v1/intents/{intent_id}
```

请求必须携带 `Idempotency-Key` 和 `If-Match`。非终态 Intent 会先安全 Abort；删除范围包含事件、关联凭据与 Scope Workspace。共享 CAS Blob 不会盲目删除，响应会明确报告保留状态与清理错误。

Runtime 设置使用：

```text
GET  /v1/settings
POST /v1/settings
```

当前设置项为 `retention_days`。`null` 表示永不自动删除；有效范围为 1–3650。自动 Retention 只删除超过期限的 Completed / Failed / Cancelled Intent。

Runtime 诊断使用：

```text
GET /v1/diagnostics
```

报告包含数据库 `PRAGMA integrity_check`、事件量、Intent 状态分布、Connector Health、Storage 占用、Credential Backend 与 Retention。不会包含 Token、凭据本体、私有 Harness 文件或绝对用户路径。

Connector 修复使用：

```text
POST /v1/intents/{intent_id}/connectors/{connector_id}/repair
```

修复会追加 `ConnectorCursorReset` 事件、清零失败退避并重新执行同步。返回同步继续失败时仍会明确报告错误，不会伪造成功。

Connector 凭据维护使用：

```text
POST   /v1/intents/{intent_id}/connectors/{connector_id}/credential
DELETE /v1/intents/{intent_id}/connectors/{connector_id}/credential
```

新凭据会替换现有引用并立即刷新 Health。撤销后 Connector 同步失败关闭，直到重新认证。

孤儿 Lease 清理使用：

```text
POST /v1/diagnostics/leases/cleanup
```

只有 RunFinished、Scope 终态或 Generation 失效的未 Revoke Lease 会被回收。Sandbox Lease 会先补 Unknown / Estimated 收尾记录，活跃 Run 会跳过。

异常 Run / Scope 安全重试使用：

```text
POST /v1/intents/{intent_id}/repair/retry
```

请求会追加 `ScopeRetryApproved`，清零失败 Scope 的失败计数，恢复 Running / Active，并以新 Generation 交回 Kernel。该操作不会直接标记完成；重试后的执行仍可能失败并进入正常收敛。

`IntentSnapshot` 是 UI 投影，不是 Runtime 权威状态：

```json
{
  "spec": { "id": "...", "outcome": "...", "done_when": [] },
  "state": "needs_decision",
  "decisions": [],
  "artifacts": [],
  "resource_budget": {},
  "resource_usage": {},
  "cursor": 42
}
```

字段名沿用 Rust 协议的 `snake_case` JSON；ID 是透明字符串，时间使用 RFC 3339。HTTP 非 2xx 响应可返回：

```json
{"error": "conflict", "message": "Decision already resolved"}
```

## 边界

- SwiftUI 只消费投影并提交命令，不持有 Runtime 执行状态。
- 网络客户端、DTO 与 Demo 实现属于 `ElseClientCore`；UI 不直接拼 JSON。
- Web 不强行复用 Swift。Web 继续使用 TypeScript，与 Apple 客户端共享同一套 Runtime API。
- 当前提供事件游标增量读取；需要更低延迟时再增加 `/v1/events/stream`，不要先让每个视图各自轮询全量状态。
