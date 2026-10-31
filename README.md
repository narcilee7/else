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
- **Attention Kernel.** Nothing may interrupt you directly. Every request is persisted, ruled on as `Silent / Defer / Batch / Interrupt`, then exposed to a surface.
- **Everything is a projection of an Event Ledger.** Restart, swap agents, or re-plan without losing why things got here. Checkpoint → Fork → Rerun → Compare operates on *task state* — artifacts, side effects, and evidence — not just code.
- **Managed tools pass through the Gateway.** Gateway-routed tools are authorized before execution. Native Harness tools that cannot be intercepted are explicitly Opaque: sandbox-confined, observed, and accounted, but not claimed as authorized.
- **One resource governance model.** Admission + Lease + Accounting covers CPU, sandboxes, API quotas, model tokens, and your attention.
- **One runtime, many surfaces.** CLI first, then Web, desktop, mobile, and IM — all thin clients of the same API. Local and cloud deployments share the same protocols and invariants.

## Documents

- [MVP](./MVP.md) — the no-scale MVP cutline: Memory, Codex / Claude Code harnesses, local dispatch, shared context and resources, followed by a separate Scale phase.（中文）
- [ARCHITECTURE](./ARCHITECTURE.md) — system design, invariants, and technology choices: Rust core, event sourcing on SQLite, pluggable artifact backends (CAS / Git), MCP + model gateways, local & cloud environments.（中文）
- [DESIGN](./DESIGN.md) — visual language, Motion grammar, component rules, accessibility, and the three-surface experience.（中文）
- [GAPS](./GAPS.md) — prioritized product, kernel, release, and scale-readiness gaps.（中文）
- [PROJECT_MANAGE](./PROJECT_MANAGE.md) — implementation rhythm: milestones M0–M4, weekly M0 breakdown, acceptance criteria, scope guards.（中文）

## Status

**MVP complete (no-scale cutline, 2026-10-30).** Single user, single machine, CLI-first. 115 tests passing on macOS.

- **Kernel (MVP-0)**: durable root Scope identity, explicit Scope dependencies, artifact lineage across child workspaces, fair multi-Scope reconciliation ordered by creation and last execution, replayable external waits and ExternalEvent/Timer wakes that feed the next Context Snapshot, continuous `drive` loops, and a resident single-thread Runtime supervisor; event-sourced ledger on SQLite, Checkpoint → Fork → Rerun → Compare, Abort + Generation Fencing, Subagent delegation tree, Admission + Lease + Accounting, deny-by-default OS sandboxes (macOS Seatbelt verified on real hardware; Linux bubblewrap policy unit-tested, awaiting Linux CI).
- **Context & Memory (MVP-1)**: Intent Resolution with revisions, three-tier memory (Working / Episodic / Semantic) with scoped promotion rules, Context Compiler producing minimal read-only snapshots per Planner / Executor / Verifier. Memory is never a source of authority.
- **Real harnesses (MVP-2)**: Codex (`codex exec --json`) and Claude Code (`claude -p stream-json`) adapters behind one bridge contract; capabilities recorded per run; usage graded Reported / Estimated / Unknown (never silent zero-cost); wall-clock timeouts; late output fenced after abort. Harness completion is only a candidate result: Runtime independently evaluates structured DoneWhen checks before marking an outcome verified. **Codex verified end-to-end live**; Claude Code adapter ready but unverified here (local login broken, 401).
- **Local dispatch (MVP-3)**: rule-based harness selection (explicit hint > task-type > availability > scripted fallback), fixed local slots (default 2, `ELSE_MAX_RUN_SLOTS`), isolated per-scope workspaces, and physical artifact materialization into the receiving Scope.
- **Publishable closure (MVP-4)**: single-command install, security regression (deny-by-default network/filesystem, no host-env credential leakage), dogfooded real tasks with attention/cost accounting.
- **Attention closure**: durable Batch / Defer windows, idempotent deadline timers, aggregated decision presentation, and replay-derived intervention metrics.
- **Structured planning**: replayable Plan revisions with evidence, explicit fact watches, Scope impact analysis, and the same `PlanProposed / PlanReplaced` bridge contract across harnesses.
- **Sandbox accounting**: conservative lease reservations separated from measured wall-clock cost, with Reported / Estimated / Unknown evidence and termination reasons.
- **Dependency convergence**: bounded retries, classified terminal failures, one same-authority replacement, explicit downstream rewiring, and replayable Superseded convergence.
- **Explicit Opaque Profile**: user-approved observed effects, provider-domain egress proxying, optional read-only Harness Workspaces with loopback Tool Gateway routing, effect ceilings, and observed events kept separate from Gateway authorization.

### Quickstart

```bash
cargo install --path surfaces/cli        # one-command install, zero external services

else intent "把项目推进到可发布状态" --project myproj
else intent "实现发布修复" --harness codex --allow-opaque --opaque-network --tool-gateway --opaque-max-effect reversible
else advance <intent-id> --harness auto     # one event-derived kernel step
else drive <intent-id> --harness auto        # reconcile until the next wait state
else event <intent-id> mail "合作方已回复"    # durable external wake
else wait-external <intent-id> --reason "等待合作方回复"
else wake <intent-id> 3600                   # durable Timer wake
else decisions <intent-id>               # what truly needs you
else attention <intent-id>               # windows, batch rate, missed critical work
else plan <intent-id>                    # current plan, evidence, and fact impacts
else allow-opaque <intent-id> --network --tool-gateway --max-effect reversible
else decide <intent-id> <request-id> approve
else run-scope <intent-id> <scope-id> --harness auto   # rule-dispatch to codex / claude-code / scripted
else memory --project myproj             # scoped recall, never permission
else resources <intent-id>               # tokens / spend / tool calls / sandbox seconds
else checkpoint <intent-id> && else rerun --alternative <intent-id> <checkpoint>
else compare <intent-id> <scope-a> <scope-b>
```

### Honest limits (Opaque capabilities & non-promises)

- **Harness tool side effects are observed, not gateway-routed.** Codex / Claude Code require an explicit Opaque Profile. Their tools are recorded as `OpaqueToolObserved`, confined to the sandbox, and never represented as Gateway-authorized calls.
- **Opaque Harness workspaces are read-only when Tool Gateway is enabled.** Controlled writes and shell calls must come through the bearer-protected loopback `else tool-call` endpoint, which applies Gateway authority, idempotency, fencing, accounting, and Sandbox execution.
- **Harness credentials live outside persistent Workspaces.** Runtime-controlled `harness-private/` directories are the only extra writable paths, are deleted after every run, and are excluded from snapshots even if legacy files remain under `.else-harness`.
- **Manual DoneWhen requires an explicit Decision.** File existence and file content are verified automatically; manual criteria create a Verification request. Approval re-checks machine criteria and completes without rerunning the Harness, while rejection leaves the Scope retryable.
- **Harness model traffic uses their native providers through a Runtime egress proxy.** Native-provider networking requires a separate user opt-in; the Sandbox only reaches the loopback proxy and the proxy enforces the adapter's provider-domain allowlist.
- **Opaque network fails closed where domain egress cannot be enforced.** The loopback-only proxy path is verified on macOS Seatbelt; Linux bubblewrap currently rejects Opaque native-provider networking rather than sharing the host network.
- **Observed-effect ceilings are enforced after observation.** Exceeding the profile kills the Harness immediately, but cannot undo a side effect that already occurred.
- **Tool Gateway routing depends on the Harness following the task contract.** Native write attempts are denied by the read-only parent sandbox, but the Runtime does not yet synthesize native provider tool-call messages into a full interactive protocol.
- **Nested sandboxes are disabled inside harnesses** (e.g. `sandbox_mode="danger-full-access"` for codex) because Seatbelt cannot nest; else's own sandbox profile is the isolation boundary.
- **Abort Succeeded means execution converged safely, not that the world was rolled back.** Irreversible effects stay irreversible and are recorded as such.
- **Misinterrupt rate is a lower bound.** It only counts Interrupt requests a user explicitly dismisses; system cancellations and rejected options are not assumed to be false alarms.
- **Plan impact is dependency-driven, not mind-reading.** External facts only invalidate steps that explicitly declare matching sources or terms; Plan Impact never grants authority or directly kills a Scope.
- **Sandbox wall-clock cost is not CPU billing.** It measures the Runtime-held sandbox lease lifetime for budget, reporting, and Compare; missing evidence is graded rather than treated as zero.
- **Dependency recovery is bounded.** A failed Scope is replaced at most once with the same authority; a failed replacement or user-cancelled upstream cuts the downstream branch instead of retrying forever.
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
