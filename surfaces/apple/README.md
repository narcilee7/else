# else Apple client

macOS 原生壳，使用 SwiftUI 与 Swift Package Manager。目标不是再做一个聊天窗口，而是把 Runtime 投影收敛成三个表面：

- 今天：正在推进的 Intent
- 待决策：只有真正需要本人的 Decision
- 结果：已收敛的 Outcome、证据与资源账

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

正式模式默认连接 `http://127.0.0.1:4180`，可通过 `ELSE_RUNTIME_URL` 覆盖。`ELSE_DEMO=1` 或 `--demo` 只用于开发与视觉验收。

## Runtime API v1

当前 Rust `api` crate 仍是进程内接口，以下 HTTP 契约是客户端与 daemon 的边界，需要由后续 `else-api` daemon 实现：

| Method | Path | Response |
| --- | --- | --- |
| `GET` | `/v1/intents` | `[IntentSnapshot]` |
| `GET` | `/v1/intents/{intent_id}` | `IntentSnapshot` |
| `POST` | `/v1/intents` | `IntentSnapshot` |
| `POST` | `/v1/intents/{intent_id}/run` | `IntentSnapshot` |
| `POST` | `/v1/intents/{intent_id}/decisions/{decision_id}/resolve` | `IntentSnapshot` |
| `POST` | `/v1/intents/{intent_id}/abort` | `IntentSnapshot` |

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
- 当前没有 SSE。需要实时刷新时再增加 `/v1/events/stream`，不要先让每个视图自己轮询。
