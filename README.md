# else-bot

**You do what matters. Else handles the rest.**

else-bot is a local-first personal agent runtime for work that spans more than one
prompt, tool, or session. You define the outcome and its boundaries; else keeps
the thread alive, decomposes it into scopes, dispatches execution, verifies the
result, and asks for your attention only when a decision belongs to you.

> The product metric is simple: how much human attention does each completed
> outcome cost?

## The Problem

Agentic tools are good at producing output. They are much worse at owning a
commitment.

A real outcome usually needs several runs, external context, approvals, retries,
and follow-up. Without a durable control plane, the work turns into a stream of
chats and notifications that you have to coordinate yourself:

- context is repeatedly lost between conversations and tools;
- plans change without a record of why;
- external changes are discovered too late;
- retries repeat side effects;
- every intermediate update competes for attention;
- the final result is difficult to verify or audit.

else-bot exists to make the **thread**, not the chat transcript or agent run, the
durable unit of work.

## What Else Does

- **Keeps thread state durable.** Plans, scopes, and agent runs are disposable;
  the outcome, constraints, decisions, and evidence survive.
- **Coordinates execution.** Codex and Claude Code can run behind one harness
  contract, with local scheduling, isolated workspaces, budgets, and retries.
- **Treats attention as a resource.** Requests are persisted and ruled as
  silent, deferred, batched, or interrupting instead of becoming notifications.
- **Tracks the outside world.** Calendar, Linear, and Notion observations become
  replayable signals and candidates rather than directly creating work.
- **Requires authority for side effects.** External actions pass through the
  Tool Gateway with approval, idempotency, verification, and compensation.
- **Exposes one runtime to every surface.** The CLI, terminal UI, and future
  clients speak the same HTTP API against the same local daemon.

## Install

### Install script

macOS and Linux:

```bash
curl -fsSL https://github.com/narcilee7/else/raw/refs/heads/master/install.sh | sh
```

The script downloads the matching release binary, verifies `SHA256SUMS`, installs
`else`, prepares the runtime directory, and runs first-run setup.

### Homebrew

```bash
brew install narcilee7/tap/else-bot
```

If Homebrew asks for explicit trust on first use:

```bash
brew trust narcilee7/tap
```

### Cargo

The first crates.io publication is still pending account authentication. Once
published, the install command is:

```bash
cargo install else-bot --locked
```

## First Run

```bash
else-bot onboard       # prepare the runtime and background service
else-bot doctor        # environment, daemon, token, ledger, and connector health
else-bot               # open the terminal interface
```

The first screen is a personal brief, not a chat window.

## Everyday Usage

Create a durable thread:

```bash
# in a directory you already own
else-bot thread "Summarize this week's meeting notes" --in ~/notes

# or in an isolated scratch directory owned by the runtime
else-bot thread "Prepare the release and verify every platform artifact" --project release
```

Inspect what needs attention:

```bash
else-bot brief
else-bot decisions <thread-id>
else-bot world <thread-id>
```

Drive work and inspect the evidence:

```bash
else-bot advance <thread-id> --harness auto
else-bot plan <thread-id>
else-bot resources <thread-id>
else-bot log <thread-id>
```

Run the shared runtime explicitly:

```bash
else-bot serve --bind 127.0.0.1:4180
```

The daemon uses a local Bearer token, remains the only writer, and lets the CLI,
TUI, and other clients observe the same state.

## How It Fits Together

```text
Thread
  -> Context and external signals
  -> Plan and execution scopes
  -> Harness or tool through the Gateway
  -> Artifact and effect verification
  -> Outcome with evidence
```

The runtime is event-sourced. Thread state, decisions, artifacts, side effects,
resource usage, retries, and verification are projections of an append-only
ledger. Replaying a ledger should reconstruct the same state.

The local-runtime boundary is deliberate:

```text
CLI / TUI / future surfaces
            |
      authenticated HTTP
            |
       else-daemon
            |
  SQLite ledger + artifacts
```

## Interfaces

The terminal UI includes:

- **Today**: decisions, active work, recent outcomes, commitments, and risks;
- **Decisions**: a review queue plus full context, options, custom responses, and
  explicit confirmation for high-impact approvals;
- **Results**: finished outcomes, acceptance criteria, and produced artifacts;
- **Settings**: connector health, credentials, and synchronization.

Thread details keep scopes, resource accounting, and raw events behind an explicit
debug toggle.

The CLI accepts `--json` globally for automation and scripting. Successful commands
write exactly one JSON document to stdout. Runtime and argument failures write
`{"error":{"kind":"api|client|usage","message":"..."}}` to stderr and return a
nonzero status. Existing read-only result shapes remain unchanged; write commands
return their resulting domain object or a named object for composite results.
`tool-call` is the exception: it preserves the Tool Gateway response body.

```bash
else-bot thread "Prepare the release" --json
else-bot decisions <thread-id> --json
else-bot --json  # non-interactive personal brief
```

## Safety Model

- execution is sandboxed by default;
- network access is deny-by-default;
- external writes require explicit authority;
- credentials are stored outside the event ledger;
- provider actions are idempotent and verified;
- opaque agent-tool effects are observed and accounted separately from
  Gateway-authorized actions;
- uncertain external results are surfaced rather than blindly retried.

else-bot is currently a single-user, single-machine runtime. It is not yet a
multi-tenant cloud service, and irreversible external actions cannot be fully
rolled back.

## Versions And Releases

Releases use semantic versions:

- stable: `v0.1.0`, `v0.2.0`, `v1.0.0`;
- beta: `v0.2.0-beta.1`, `v0.2.0-beta.2`;
- release candidate: `v0.2.0-rc.1`.

`0.x` is initial development: interfaces may still change between minor
versions. `1.0.0` marks the first compatibility promise.

Beta and RC tags are published as GitHub prereleases. They are not marked
`Latest` and do not update the Homebrew stable formula.

## Platform Support

| Platform | CLI/TUI |
| --- | --- |
| macOS arm64 | Supported |
| macOS x86_64 | Supported |
| Linux arm64 | Supported |
| Linux x86_64 | Supported |

macOS CLI archives are ad-hoc signed until Developer ID credentials are
available. The macOS App is not part of the current public release line.

## Development

Use the isolated development runtime rather than the installed background service.
It lives in `.else/dev`, listens on `127.0.0.1:4181`, and never requires onboarding
or modifying the production runtime. Every command builds the current source first.

Start the daemon in one terminal:

```bash
./scripts/dev.sh serve
```

Populate deterministic demo data and open the TUI in another terminal:

```bash
./scripts/dev.sh seed
./scripts/dev.sh tui
```

The seed command is idempotent and creates a pending decision, a completed result,
and work waiting for an external event. It uses the built-in scripted harness, so
it does not need provider credentials or network access. Python 3 is required for
seeding and reset safety checks.

Run CLI commands against the same development runtime:

```bash
./scripts/dev.sh cli list
./scripts/dev.sh cli brief --json
./scripts/dev.sh cli decisions <thread-id>
```

After changing only the TUI, rebuild and reopen it; the daemon can stay running.
After changing the API, daemon, or shared protocol, restart both processes. Stop
the daemon before clearing the development data:

```bash
./scripts/dev.sh reset
```

For another isolated checkout or runtime, set `ELSE_DEV_RUNTIME_DIR` and optionally
`ELSE_DEV_BIND`. Reset refuses paths outside this repository's `.else` directory
and refuses to delete a runtime whose foreground daemon is still running.

Run the complete Rust validation suite:

```bash
./scripts/dev.sh check

cd surfaces/apple
swift test
```

Build a local CLI artifact:

```bash
scripts/release.sh 0.1.3
```

Build internal release artifacts without Apple credentials:

```bash
gh workflow run release-cli.yml -f version=0.1.3 -f publish=false
```

## License

[MIT](./LICENSE)
