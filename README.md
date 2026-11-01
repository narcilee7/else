# else

**You do what matters. Else handles the rest.**

else is a local-first personal agent runtime for work that spans more than one
prompt, tool, or session. You define the outcome and its boundaries; else keeps
the intent alive, decomposes it into scopes, dispatches execution, verifies the
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

else exists to make the **intent**, not the chat transcript or agent run, the
durable unit of work.

## What Else Does

- **Keeps intent state durable.** Plans, scopes, and agent runs are disposable;
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
curl -fsSL https://raw.githubusercontent.com/narcilee7/else/master/install.sh | sh
```

The script downloads the matching release binary, verifies `SHA256SUMS`, installs
`else`, prepares the runtime directory, and runs first-run setup.

### Homebrew

```bash
brew install narcilee7/tap/else
```

If Homebrew asks for explicit trust on first use:

```bash
brew trust narcilee7/tap
```

### Cargo

The first crates.io publication is still pending account authentication. Once
published, the install command is:

```bash
cargo install else-cli --locked
```

## First Run

```bash
else onboard       # prepare the runtime and background service
else doctor        # environment, daemon, token, ledger, and connector health
else               # open the terminal interface
```

The first screen is a personal brief, not a chat window.

## Everyday Usage

Create a durable intent:

```bash
else intent "Prepare the release and verify every platform artifact" --project release
```

Inspect what needs attention:

```bash
else brief
else decisions
else world
```

Drive work and inspect the evidence:

```bash
else advance <intent-id> --harness auto
else plan <intent-id>
else resources <intent-id>
else log <intent-id>
```

Run the shared runtime explicitly:

```bash
else serve --bind 127.0.0.1:4180
```

The daemon uses a local Bearer token, remains the only writer, and lets the CLI,
TUI, and other clients observe the same state.

## How It Fits Together

```text
Intent
  -> Context and external signals
  -> Plan and execution scopes
  -> Harness or tool through the Gateway
  -> Artifact and effect verification
  -> Outcome with evidence
```

The runtime is event-sourced. Intent state, decisions, artifacts, side effects,
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

- **Overview**: brief, decisions, risks, active work, and upcoming items;
- **Intents**: durable outcomes and their current state;
- **World**: commitments, waiting-external work, action items, and risks;
- **Decisions**: the small set of requests that genuinely need the user;
- **Connectors**: provider health, credentials, and synchronization.

The CLI also exposes structured commands for automation and scripting. Read-only
commands support `--json` where available.

## Safety Model

- execution is sandboxed by default;
- network access is deny-by-default;
- external writes require explicit authority;
- credentials are stored outside the event ledger;
- provider actions are idempotent and verified;
- opaque agent-tool effects are observed and accounted separately from
  Gateway-authorized actions;
- uncertain external results are surfaced rather than blindly retried.

else is currently a single-user, single-machine runtime. It is not yet a
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

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace

cd surfaces/apple
swift test
```

Build a local CLI artifact:

```bash
scripts/release.sh 0.1.0
```

Build internal release artifacts without Apple credentials:

```bash
gh workflow run release-cli.yml -f version=0.1.0 -f publish=false
```

## License

[MIT](./LICENSE)
