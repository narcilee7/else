# else

**You do what matters. Else handles the rest.**

else is a **managed, personal agent runtime** — an *Intent Runtime*.
You define the outcome and the boundaries; else handles decomposition, scheduling, execution, follow-up, and re-planning — and only asks for your attention when a decision is truly yours.

Not another multi-agent desktop UI. Not a notification center. Not a chat-memory search box. Not yet another agent framework.

## Why

Coordinating agents is becoming a job of its own: checking progress, re-explaining context, cleaning up mistakes. else treats **human attention as the scarcest resource** and optimizes for one metric:

> How much of your attention does each completed outcome cost?

## Core ideas

- **Intent is a first-class citizen.** Plans, tasks, and agents are disposable, rebuildable internals. Plans are volatile; Intent State is durable.
- **Attention Kernel.** Nothing may interrupt you directly. Every request is ruled on: `Silent / Defer / Batch / Interrupt`.
- **Everything is a projection of an Event Ledger.** Restart, swap agents, or re-plan without losing why things got here. Checkpoint → Fork → Rerun → Compare operates on *task state* — artifacts, side effects, and evidence — not just code.
- **Agents own no permissions.** All side effects go through a Tool Gateway; all model calls through a Model Gateway. One resource governance model (Admission + Lease + Accounting) covers CPU, sandboxes, API quotas, model tokens — and your attention.
- **One runtime, many surfaces.** CLI first, then Web, desktop, mobile, and IM — all thin clients of the same API. Local and cloud deployments share the same protocols and invariants.

## Documents

- [MVP](./MVP.md) — the no-scale MVP cutline: Memory, Codex / Claude Code harnesses, local dispatch, shared context and resources, followed by a separate Scale phase.（中文）
- [ARCHITECTURE](./ARCHITECTURE.md) — system design, invariants, and technology choices: Rust core, event sourcing on SQLite, pluggable artifact backends (CAS / Git), MCP + model gateways, local & cloud environments.（中文）
- [DESIGN](./DESIGN.md) — visual language, Motion grammar, component rules, accessibility, and the three-surface experience.（中文）
- [PROJECT_MANAGE](./PROJECT_MANAGE.md) — implementation rhythm: milestones M0–M4, weekly M0 breakdown, acceptance criteria, scope guards.（中文）

## Status

**MVP complete (no-scale cutline, 2026-10-30).** Single user, single machine, CLI-first. 69 tests passing on macOS.

- **Kernel (MVP-0)**: event-sourced ledger on SQLite, Checkpoint → Fork → Rerun → Compare, Abort + Generation Fencing, Subagent delegation tree, Admission + Lease + Accounting, deny-by-default OS sandboxes (macOS Seatbelt verified on real hardware; Linux bubblewrap policy unit-tested, awaiting Linux CI).
- **Context & Memory (MVP-1)**: Intent Resolution with revisions, three-tier memory (Working / Episodic / Semantic) with scoped promotion rules, Context Compiler producing minimal read-only snapshots per Planner / Executor / Verifier. Memory is never a source of authority.
- **Real harnesses (MVP-2)**: Codex (`codex exec --json`) and Claude Code (`claude -p stream-json`) adapters behind one bridge contract; capabilities recorded per run; usage graded Reported / Estimated / Unknown (never silent zero-cost); wall-clock timeouts; late output fenced after abort. **Codex verified end-to-end live**; Claude Code adapter ready but unverified here (local login broken, 401).
- **Local dispatch (MVP-3)**: rule-based harness selection (explicit hint > task-type > availability > scripted fallback), fixed local slots (default 2, `ELSE_MAX_RUN_SLOTS`), isolated per-scope workspaces, artifact handoff via snapshot references.
- **Publishable closure (MVP-4)**: single-command install, security regression (deny-by-default network/filesystem, no host-env credential leakage), dogfooded real tasks with attention/cost accounting.

### Quickstart

```bash
cargo install --path surfaces/cli        # one-command install, zero external services

else intent "把项目推进到可发布状态" --project myproj
else decisions <intent-id>               # what truly needs you
else decide <intent-id> <request-id> approve
else run-scope <intent-id> <scope-id> --harness auto   # rule-dispatch to codex / claude-code / scripted
else memory --project myproj             # scoped recall, never permission
else resources <intent-id>               # tokens / spend / tool calls / sandbox seconds
else checkpoint <intent-id> && else rerun --alternative <intent-id> <checkpoint>
else compare <intent-id> <scope-a> <scope-b>
```

### Honest limits (Opaque capabilities & non-promises)

- **Harness tool side effects are observed, not gateway-routed.** Codex / Claude Code execute their own tools; else records them as observed events with conservative effect levels and confines them to the sandbox. They are *accounted*, not *authorized*.
- **Harness model traffic uses their native providers with explicit outbound network.** Credentials are staged into the workspace (`CODEX_HOME` / `CLAUDE_CONFIG_DIR`) and the sandbox allows outbound traffic for these runs — a documented Opaque boundary, not a leak. Egress proxying is post-MVP.
- **Nested sandboxes are disabled inside harnesses** (e.g. `sandbox_mode="danger-full-access"` for codex) because Seatbelt cannot nest; else's own sandbox profile is the isolation boundary.
- **Abort Succeeded means execution converged safely, not that the world was rolled back.** Irreversible effects stay irreversible and are recorded as such.
- **Platform**: macOS Seatbelt is verified on real hardware; Linux bubblewrap passes policy unit tests but awaits Linux CI runtime verification. No Windows backend.
- **No scale**: single user, single machine, ≤2 local run slots. Cloud, multi-user, and remote workers are a separate post-MVP phase (see [MVP](./MVP.md) §11).

## Roadmap

| Milestone | Focus |
| --- | --- |
| M0 | Core loop: protocols, event ledger, gateways, one harness, CLI |
| M1 | Execution semantics: abort, generation fencing, subagents, resource governance, local sandboxing |
| M2 | Context & memory: intent resolution, layered memory, context compiler |
| M3 | Cloud & surfaces: cloud runtime and sandboxes, model routing, Web + desktop |
| M4 | Reach & collaboration: IM, mobile, team spaces |

## License

See [LICENSE](./LICENSE).
