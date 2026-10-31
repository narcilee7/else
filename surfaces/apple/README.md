# else Apple client

macOS 原生壳，使用 SwiftUI 与 Swift Package Manager。目标不是再做一个聊天窗口，而是把 Runtime 投影收敛成三个主表面与常驻体验：

- 今天：可解释 Priority、来源证据、分组 Personal Brief、外部事项、进行中 Intent 与 Decision 聚合
- 待决策：只有真正需要本人的 Decision，按 Batch/Defer Window 聚合，可直接完成常用选项或进入完整 Intent
- 结果：已收敛的 Outcome、证据与资源账；详情含 ConnectorAction 执行、核验、补偿与最终副作用结论
- 常驻体验：15/30/60 秒后台刷新、去重系统通知、Dock Badge、菜单栏 Quick Access 与设置页

视觉、Motion 与组件约束见 [DESIGN.md](../../DESIGN.md)。

## 结构

```text
surfaces/apple/
├── Sources/ElseClientCore/   # 协议 DTO、RuntimeClient、HTTP 传输、Demo 实现
├── Sources/ElseMac/          # SwiftUI 应用、状态容器与本地配置
├── Tests/ElseClientCoreTests/
├── Resources/Info.plist
└── scripts/build-app.sh
```

`ElseClientCore` 不依赖 SwiftUI，后续 iOS、菜单栏、Widget 与 App Intents 可复用。客户端不直接链接 Rust `Runtime`，只通过版本化 Runtime API 工作。

## 本地运行

```bash
cd surfaces/apple
swift test

# 不需要 daemon 的 UI 开发模式
swift run ElseMac --demo

# 生成可启动的 .app bundle
./scripts/build-app.sh
open .build/Else.app --args --demo
```

正式模式默认连接 `http://127.0.0.1:4180`，可通过 `ELSE_RUNTIME_URL` 覆盖；Bearer Token 从 `ELSE_RUNTIME_TOKEN` 读取。`ELSE_DEMO=1` 或 `--demo` 只用于开发与视觉验收。

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
