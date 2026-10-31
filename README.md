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
- [PERSONAL_AGENT_PROGRESS](./PERSONAL_AGENT_PROGRESS.md) — Personal Agent 产品阶段、出口条件、阻塞项与迭代记录。（中文）
- [CONNECTORS](./CONNECTORS.md) — World Signal / Intent Candidate / Provider Connector abstraction and rollout plan.（中文）
- [PROJECT_MANAGE](./PROJECT_MANAGE.md) — implementation rhythm: milestones M0–M4, weekly M0 breakdown, acceptance criteria, scope guards.（中文）

## Status

**MVP complete (no-scale cutline, 2026-10-30).** Single user, single machine, CLI-first. 153 Rust tests and 9 Swift tests passing on macOS.

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
- **Loopback daemon**: single-instance runtime, Bearer authentication, idempotent commands, revision preconditions, asynchronous operations, and cursor-based event streaming for CLI, Apple, and future surfaces.
- **Unified capabilities and bindings**: Builtin tools, MCP tools, and Skills share one Gateway contract; MCP tools can be discovered from `tools/list`, while LocalProcess, McpStdio, and HTTPS RemoteHttp Agent bindings dispatch outbound work through the existing verifier and resource lifecycle.
- **Binding lifecycle closure**: MCP sessions support `list_changed` refresh and persisted restart recovery; Agent bindings expose explicit lifecycle states, RemoteHttp callbacks, and SSE event streams.
- **Personal world signal foundation**: an Adapter Registry isolates Calendar, Linear, and Notion provider details behind neutral Observations; a separate Interpreter derives ActionItem / Commitment / WaitingExternal interpretations before Resolver sees them. Notion Pages enter Document Context while Database Rows reuse the WorkItem pipeline. Connectors retain manifests, persistent cursors, tombstones, and session-scoped webhooks.
- **Live connector foundation**: OAuth/API credentials live in macOS Keychain or a 0600 private runtime store, never the Event Ledger. Scoped credentials refresh before expiry, can be revoked, and feed a resident scheduler with persistent health, exponential backoff, and `Retry-After` handling.
- **Personal World**: commitments, waiting-external items, actions, overdue work, and risks are aggregated from replayable interpretations. Users can resolve items without pretending the external provider changed, and a cross-Intent brief summarizes open loops.
- **Project World**: provider-neutral correlation hints group Linear, Notion, and Calendar items by normalized project key across intents, with open / waiting / overdue counts and the nearest deadline.

### Quickstart

```bash
cargo install --path surfaces/cli        # one-command install, zero external services

else serve --bind 127.0.0.1:4180          # shared loopback Runtime for Apple/Web/remote CLI
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
else calendar <intent-id> --source ./calendar.ics
else linear <intent-id> --source ./linear.json
else linear <intent-id> --source https://api.linear.app/graphql --token-env LINEAR_API_TOKEN
else notion <intent-id> --database <database-id-or-url> --token-env NOTION_TOKEN
else notion <intent-id> --token-env NOTION_TOKEN   # search all databases/pages shared with the integration
else notion <intent-id> --database <database-url> --token-keychain-service else-notion
else connector-adapters
else connector <intent-id> notion.api ./notion.json
else connector-policy <intent-id> --auto-accept --allow-origin commitment --min-confidence 80
else connectors <intent-id>
else connector-action-propose <intent-id> <connector-id> <capability> <target> --input '{"title":"..."}'
else connector-action-execute <intent-id> <action-id>
else connector-actions <intent-id>
else connector-credential-set <intent-id> <connector-id> --token-keychain-service else-notion
else connector-credential-revoke <intent-id> <connector-id>
else signals <intent-id>
else signal-feed --provider linear --cursor 0
else resolve-signals <intent-id>
else candidates <intent-id>
else candidate-feed --cursor 0
else world <intent-id>
else world-resolve <intent-id> <item-id> completed
else brief
else projects
else projects <intent-id>
else candidate-accept <intent-id> <candidate-id>
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
- **Loopback HTTP is the shared surface boundary.** `/v1/health` is public on loopback; every Runtime endpoint requires the token from `daemon-token` or `ELSE_RUNTIME_TOKEN`.
- **RemoteHttp is a protocol, not a vendor adapter.** It uses HTTPS + NDJSON/callback and SSE for Runtime-side consumption. MCP stdio sessions are recreated rather than migrated across machines.
- **Connectors remain read-only.** Calendar, Linear, and Notion Signals have versions, source, validity, tombstones, manifests, persisted pagination/backfill cursors, Keychain-backed credentials, and a resident scheduler. Linear webhooks can hydrate ID-only events; contacts and richer entity merging are not yet modeled. Personal / Project World aggregate cross-provider items and support explicit user resolution; Resolver output remains Candidate-only unless a user accepts it or enables a narrow low-risk policy.
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
