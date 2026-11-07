# TUI 设计

这不是"界面美化"。原来的终端界面在一件事上失败了：**它把一个有时间线、有人要做决定的工作，画成了一个仪表盘。**

## 0. 先看了别人怎么做

设计之前读了两个成熟实现的源码，不是因为它们好看，而是因为它们在同一个问题上已经付过学费。

**Codex**（`openai/codex`，`codex-rs/tui`）：

- `bottom_pane/mod.rs` 的 `BottomPane` 不是一排固定分栏，而是**一个保留着的输入框 + 一叠能自我折叠的行**：运行状态、横幅、待输入的队列、别处待审的请求，各自是一个 widget，**没话说时高度为 0**。整屏因此永远只显示此刻真有的东西。
- 同一文件里 `view_stack: Vec<Box<dyn BottomPaneView>>`：弹窗、审批、选择器都是**压在当前界面上的视图**，而不是和主界面并列的一屏。所以从头到尾只有一张主表面。
- `bottom_pane/pending_thread_approvals.rs`：别的会话里等你批准的请求，**最多列 3 条**，然后 `...`，再一行暗色提示 `"/subagents to review requests"`。这就是"截断 + 指路"，而不是把队列堆到屏幕上。
- `bottom_pane/footer.rs`：`FooterMode` 是个枚举（搜索中 / 再按一次退出 / 有草稿 / 空输入…），`FooterKeyHints` 的每个字段都是 `Option`。**页脚不是键位表，是"此刻能按什么"。** 全量键位在 `?` 后面。
- `bottom_pane/action_required_title.rs`：需要人处理时把 `[ ! ] Action Required` 写进**终端窗口标题**。人不在这个标签页里的时候，那行字就是通知。
- `bottom_pane/status_line_content.rs`：顶部/底部是一条由 `STATUS_LINE_SEPARATOR` 连接的**段列表**（目录、模型、token、分支），不是一张说明卡。

**herdr**（`herdrdev/herdr`，终端里的 agent 工作台）：

- `src/detect/mod.rs` 的 `AgentState` 只有四个值：`Idle / Working / Blocked / Unknown`，其中 `Blocked` 的定义就是"需要人输入、正卡在那里"。**整个状态模型就这么多。**
- `src/terminal_notify.rs`：状态变化通过终端原生通知（识别 Ghostty / iTerm2 / Kitty / WezTerm）。通知是循环的一部分，不是额外功能。

提炼成三条规矩，这份设计就是它们的三次应用：

1. **一张主表面，其余都是压在上面的视图。** 没有标签页。
2. **一行只在该出现的时候出现，最多三行，超出的部分给出通往全部的键。**
3. **页脚按当前模式给键，`?` 才展开全部。**

## 1. 原来的具体问题

1. **首屏是仪表盘。** 两栏堆了 8 个分组（Needs you / Active / Recently completed / Commitments / Due soon / Upcoming / Projects / Relationships）。用户要自己找"现在关我什么事"。
2. **每条占两行。** 标题一行、说明一行，一屏放不下六件事。
3. **决策离开了上下文。** 列表一帧、详情一帧，选完回到列表。要判断"该不该批"，得记住刚才看的东西。
4. **看不见工作本身。** Thread 详情只有目标、计划、验收和最后几条事件 **id**。Agent 刚说了什么、动了哪个文件、花了多少 —— 不在界面上。
5. **导航是开发者视角。** `1 Today / 2 Decisions / 3 Results / 4 Settings` 是代码结构，不是用户的任务结构；外面还挂着 composer / decision detail / thread detail 三个隐形屏幕。
6. **状态不可见。** 常驻 Runtime 是否在跑、这条命令用的是不是临时替身 —— 这些只在 `doctor` 里。

## 2. 结构

屏幕只有三层，代码里也是三层（`views::render`）：

```
┌ 状态条  1 行   else · 我在哪            右边：几个在等人 · 几个在跑 · 是否临时 Runtime
├ 内容    若干行 一列从上往下读的行。没有边框，没有分栏。
└ 页脚    2 行   能按什么 / 刚发生了什么
```

**没有边框。** 画边要吃掉 2 行 2 列，换来的只是一个标题 —— 而分组标题本来就能自己说话。

**没有分栏。** 分栏意味着两件事同样重要；而"该看哪一个"正是界面该替用户决定的事。

## 3. 信息架构

| 屏幕 | 回答的问题 | 入口 |
| --- | --- | --- |
| **Now** | 现在什么在等我？ | 默认 |
| **Thread** | 这件事到底怎么样了？ | 任一行 `⏎` |
| **Decisions** | 有哪些事挂着？ | `d` |
| **All threads** | 所有的都在哪？ | 首屏 `+N more` |
| **New** | 我要它做一件事 | `n` |
| **Connections** | 外部来源通不通？ | `c` |
| **Review** | 这一条到底该不该批？ | 在 Decisions 里 `⏎` |

复核（Review）是唯一"两屏"的地方，因为它必须回答"预期损失多大"，而那是首屏不承担的信息。

### Now

```text
else · now                                          2 waiting · 2 running

 NEEDS YOU
 ▸ Approve running a shell command                          high · 3f2a91c
 ▸ Overwrite launch/outline.md                              high · 3f2a91c
 +1 more · d reviews all of them

 RUNNING
 ● Rewrite the pricing page                             scratch · 8.2k tok
 ● Write the weekly report                              scratch · 1.1k tok

 RECENT
 ✓ Fix the CSV export                              3 artifacts · done

 WORLD
 · Customer sync                                            due 10-31 15:00

⏎ open  d decisions  n new  ? keys
```

规则：

- 四个分组，顺序就是"该不该你管"：**NEEDS YOU → RUNNING → RECENT → WORLD**。
- 每组**最多三行**，超出只报数量，并给出下一步（`d`，或 `+N more` 本身就是可选项）。
- 每行**一行说完**：左边是什么事，右边是拿来判断它的数字。
- 列表不为了塞下理由而变成两行；理由在**选中那一行时**出现在页脚第二行（`Row::note`）。
- 没有数据时不摆空表格，直接给下一个动作。

### Thread

```text
else · thread 3f2a91c                            needs you · runtime temp

 Prepare the launch deck

 works in scratch · revision 1

 ⚠ needs you · Overwrite launch/outline.md?                high impact
   it changes a system outside the workspace · waiting is not reversible
   ▸ Allow once (recommended)
   e writes a different answer · x dismisses

 TIMELINE
 09:31  ▸ started   claude-code run
 09:32  · said      Reading launch/*.md — six files
 09:33  · model     MiniMax-M3 · 11363 in / 3 out
 09:34  · tool      shell (reversible)
 09:35  ⚠ needs you asked you · still waiting above

 RESULT
 + launch/outline.md
 · the deck exports as PDF

⏎ open  a advance  e answer  d debug  Esc back  ? keys
```

规则：

- 顶部是**事实**：目标、在哪做、改过几版。目标只出现一次（正文标题），状态条只回答"我在哪一个"。
- **待你决定的事排在最前面**，选项就是可执行的行，按 `⏎` 即生效 —— 判断和决定之间不切屏。
- 时间线是**账本事件的用户可读投影**（`thread_detail::Entry::project`）。账本里有七十多种事件，界面只投影用户要读的那十几种；其余在 `d` 之后。
- 事件的时间、动作、正文三栏对齐，扫一眼就知道"什么时候、干了什么"。
- 已经挂在上面等回答的那条请求，在时间线里不再重复它的正文。
- 事件 id、scope、generation 不进主视觉。

### New

```text
else · new thread

 What should else take care of?

 > Finish the launch deck▏

⏎ create  Esc cancel  ? keys
```

### 复核（Review）

`WAITING ON YOU` 里 `⏎` 进去，只有一屏：这要你决定什么、为什么它自己不能决定、影响多大、你的选项。高风险批准需要再确认一次。

## 4. 视觉语言

- 字形：`▸` 动作或计划 · `●` 在跑 · `✓` 完成 · `✗`/`×` 失败 · `…` 等人 · `⚠` 需要你 · `+` 产物 · `·` 普通。全局一致。
- 颜色按语义（`theme`）：红=急、黄=风险、蓝=等人、绿=完成、灰=元数据。
- 左边距统一 **1 格**；分组标题、行、说明都在同一根竖线上。
- 右对齐的 `meta` 列固定贴右边，同类数字竖着可比。
- 带 `meta` 的行**不折行**（折了就不齐）；不带宽度的正文行才折行。

## 5. 键盘

| 键 | 作用 |
| --- | --- |
| `j`/`k` `↑`/`↓` `PgUp`/`PgDn` `Home`/`End` | 在**可执行的行**之间移动（分组标题不可选） |
| `⏎` | 打开 / 选择 |
| `Esc` | 返回上一层 |
| `n` | 新建 Thread |
| `d` | 决策队列；在 Thread 里是调试信息 |
| `a` | 让 Runtime 推进 |
| `e` | 自己写一个回答 |
| `x` | 停掉这次执行（会二次确认）；在复核里是忽略这条请求 |
| `/` | 命令面板：搜「此刻能做的事」 |
| `c` | 连接器 |
| `r` | 重新加载 |
| `?` | 全部键位 |
| `q` / `Ctrl-C` | 退出 |

页脚最多 6 项，按屏幕变化；`?` 永远在最后。

## 6. 状态与诚实

- **临时 Runtime**：状态条常驻 `runtime temp`（`Session::embedded()` 为真时）。
- **窗口标题**：有请求等人时是 `[!] N waiting · else` —— 切到别的标签页也不会错过（照 `action_required_title.rs` 的做法）。
- **加载**：状态条显示 `syncing`，不遮内容。
- **失败**：一行原因 + 一个可执行动作，不弹窗。

## 7. 测试

- **帧快照**（`views::tests`，`TestBackend`）：断言的是**结构**而不是像素 —— 分组顺序、每组上限、时间线内容、边界尺寸不 panic。
- **PTY**（`tests/tui_pty.rs`）：冷启动走真实 PTY、内嵌 Runtime 存活、空状态文案、`n` 建 Thread 后进程干净退出。因为增量重绘会把一行拆成几段写入，PTY 里只认单词，排版由帧测试负责。
- **状态机**（`app::tests`）：选择只落在可执行行、刷新后按业务 id 恢复选择、高风险批准必须二次确认。

### 命令面板

`/` 打开。它**就是页脚的展开**：列出来的都是此刻按对应键真的会发生的事，
右边还标着那个键。做不到的事不列进来 —— 面板不承诺它执行不了的操作。

```text
┌ / ─────────────────────────────────────────────┐
│> quer                                          │
│                                                │
│Describe a new thread                           │
│Reload from the runtime                        r│
│Write your own answer                          e│
│Let the runtime advance this thread            a│
│Stop this run                                  x│
│Quit                                           q│
│↑↓ move  ⏎ run  Esc cancel                       │
└────────────────────────────────────────────────┘
```

## 8. 通知

三处，按"离你的远近"排：

1. **窗口标题**：`[!] N waiting · else`。切到别的标签页时唯一还在的提醒。
2. **终端原生通知**：新请求到达时发一条（iTerm2 / Ghostty / WezTerm / kitty）。
   识别不出终端就不发 —— 不往未知终端里塞转义序列。
3. **状态条计数**：一直是当下真实的数字。

只对**新来的**请求发通知：第一次加载是数据，不是打扰。

## 9. 还没做的

- 实时流式输出：agent 说话是逐行落账的，但界面一次投影整屏。
- Context 不读目标目录内容（`else-context` 里没有 `read_dir`）。
- ACP 没接：权限通道仍关着，只能事后看见 `opaque_tool_observed`。
- Harness 调度只有「不抢用户」。

## 不做的事

- 不做多栏仪表盘、不做卡片墙。
- 不把聊天框当主界面：会话属于 Run，Thread 是承诺。
- 不在主视觉里暴露事件 id、scope、generation。
