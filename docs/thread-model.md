# Thread 模型

这份文档定义 else 的产品词汇与对象模型，以及它和 harness、权限、观测的关系。
代码先按这里改；有冲突时以此为准，改模型要改这份文档。

## 1. 为什么叫 Thread

"Intent" 是设计者的词，不是用户的词。用户想的是"我有一件事要它做"，
而且这件事会持续、会被追问、会攒出上下文 —— 那个东西叫 Thread。

但 Thread 很容易被理解成聊天记录，所以先把边界钉住：

> **Thread 不是聊天。** Thread 是承诺：目标、验收、约束、权限与证据。
> 聊天（一轮对话）是 Run 内部的事，可以是多次，可以是不同 Agent。

聊天记录丢了可以重开；Thread 丢了就是工作丢了。这就是为什么状态存在账本里，
而不是存在于某个 Agent 的会话里。

## 2. 对象模型

```text
Thread   一件事。目标 + 验收 + 约束 + 权限 + 记忆作用域
  └─ Scope   一个可交付的分支（可以并行、可以依赖、可以委托）
       └─ Run      一次执行：选一个 harness、一个沙箱、一个上下文快照
            └─ Session   Agent 自己的会话（ACP session / CLI 进程内）
```

- **Thread** 是唯一的持久单元，跨天、跨 Agent、跨重启。
- **Run** 是易失的：换 Agent 不换 Thread。
- **Session** 完全归 harness 所有，else 不把它当权威状态。
- `revision`（目标语义版本）与 `generation`（执行围栏）继续不混用。

术语对应（改名前 → 改名后）：`Intent` → `Thread`，`IntentSpec` → `ThreadSpec`，
`user_intent` → `user_thread`，HTTP `/v1/intents` → `/v1/threads`。

## 3. 不绑定 worktree：Thread 有"执行位置"，但没有假设那是代码

else 是个人 Agent，不是编码 Agent。绑定 git worktree 会把产品缩成一种模式。
Thread 只声明**在哪里做**，由 Mode 决定：

| Mode | 执行位置 | 典型用途 |
| --- | --- | --- |
| `scratch` | runtime 下的隔离目录（默认） | 研究、写作、整理、一次性任务 |
| `directory` | 用户指定的目录（不只代码） | 文档、资料库、任意真实目录 |
| `repo` | 目录 + git 分支/worktree | 代码改动、可回滚的提交 |
| `remote` | 远端（云沙箱 / 另一台机器） | 长时间任务、重活 |

规则：

- Mode 是 Thread 的属性，不是全局设置；同一个 Runtime 里可以同时存在四种。
- 只有 `repo` 才谈分支、diff、合并；`scratch` 交付的是工件（artifact）。
- 工件是唯一跨 Mode 的交付物：文件树快照，内容寻址，不绑定 Git。
- 验收（DoneWhen）永远相对执行位置判断，不假设"代码能编译"。

## 4. Agent 层：宿主式（ACP 思路），不是黑盒进程

现在的适配器是"起一个 CLI，扫它的 stdout"：`claude -p ... --permission-mode
acceptEdits --permission-prompts none`。这等于**主动关掉权限通道**，只能事后记账。

改成宿主式：else 是 Client，Agent 是子进程，双方按 ACP 通信。

| ACP | else 侧已有 | 要做 |
| --- | --- | --- |
| `session/request_permission` | `GatewayError::NeedsApproval` → AttentionRequest | 接上，权限请求落账 |
| `fs/read_text_file` `fs/write_text_file` | Tool Gateway（幂等/核验/补偿） | 落账后回写 |
| `terminal/*` | else-environment 沙箱 | 输出回流事件 |
| `session/update`（chunk/plan/mode/compaction） | `ModelCalled` / `StructuredPlanProposed` | 补 transcript 事件 |
| `session/load` | （缺） | Thread 内可续会话 |

关键点：**Agent 不直接写文件、不直接跑命令**，它请求，else 执行并落账。
这样权限、幂等、补偿、审计才成立 —— 这也是 else 相对其它 harness 的差异点。

## 5. 权限与注意力

一条授权请求的生命周期：

```text
Agent ──request_permission──▶ Gateway ──NeedsApproval──▶ AttentionRequest
                                                            │
                            Decision（批准/拒绝/改路径/自定义）◀┘
```

要求：

- 决策展示"要做什么"（工具 + 参数摘要 + 影响范围），不是"批准执行 shell"。
- 高风险与可逆操作走不同默认；重复同类授权可以"这个 Thread 内不再问"。
- 没有送达渠道的注意力等于没有注意力：CLI/TUI 至少要有本地通知。

## 6. 观测：没有 transcript 的产品不算 Agent

当前 Intent 详情只有"最后 12 条事件的 id"。Thread 必须能回答"它到底做了什么"：

- 每个 Run 一条可读时间线：assistant 文本、工具调用及结果、计划变更、压缩点。
- 事件是权威，视图是投影：TUI/CLI/App 读同一份事件，不各自造状态。
- `else-bot log <thread>` 必须能用（现在 404）。

## 7. 兼容与迁移

改名不重写历史：

- 老账本（0.1.x）打开时把列 `intent_id`/`intent_revision` 原地改名，事件内容一条不动。
- payload 里的旧字段与旧事件名（`intent_created` 等）由 serde alias 兼容。
- daemon 同时接受 `/v1/intents` 与 `/v1/threads`，旧客户端不会立刻坏。
- CLI 保留 `else-bot intent` 作为 `thread` 的隐藏别名（肌肉记忆）。
- deep link 改用 `else://thread/{id}`；旧 `else://intent/{id}` 不再接受（0.x 可接受）。

## 8. 分阶段

已完成（本分支）：

1. 词汇改名：协议类型、wire、CLI、TUI、App、README；老账本迁移 + 兼容测试。
2. 核心命令的帮助文案改成用户语言。

接下来（每步单独可评审）：

3. `ThreadMode`（`scratch` / `directory`）落到模型与执行路径；`repo` 后置。
4. ACP 宿主适配器：先接 claude-code，`request_permission` → Decision。
5. Run transcript 事件 + TUI Run 视图；修 `log`。
6. 医生说真话：区分常驻 daemon 与临时 embedded；LaunchAgent 落日志。
7. 通知通道（先 `osascript`/`terminal-notifier`）。

## 不做的事

- 不做"只服务编码"的模式：Mode 是属性，不是产品定位。
- 不把 Thread 变成聊天窗口：会话属于 Run。
- 不用后台 reconciler 替用户决定 harness；回落必须落账并可见。
