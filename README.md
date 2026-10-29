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

- [ARCHITECTURE](./ARCHITECTURE.md) — system design, invariants, and technology choices: Rust core, event sourcing on SQLite, pluggable artifact backends (CAS / Git), MCP + model gateways, local & cloud environments.（中文）
- [PROJECT_MANAGE](./PROJECT_MANAGE.md) — implementation rhythm: milestones M0–M4, weekly M0 breakdown, acceptance criteria, scope guards.（中文）

## Status

**M0 in progress — W2 done.** Thin Tool Gateway (side-effect authority, idempotency, workspace fencing) and Model Gateway (token/cost accounting), a scripted harness, and the runtime API + CLI are working: `else intent "..."` executes end-to-end and stops at a human decision when authority is exceeded. 11 tests passing, clippy clean. Next: automatic resume after decisions, re-planning, and DoneWhen verification (W3).

M0 delivers one vertical slice through the CLI: an abstract intent → execution → a decision only you can make → automatic re-planning → verification against the original outcome — plus Checkpoint / Fork / Rerun / Compare. No dashboards, no cloud, no multi-user.

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
