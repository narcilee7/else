#!/usr/bin/env bash
# Isolated, source-built development workflow for else-bot.

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
RUNTIME_DIR="${ELSE_DEV_RUNTIME_DIR:-$ROOT/.else/dev}"
BIND="${ELSE_DEV_BIND:-127.0.0.1:4181}"
BINARY="$ROOT/target/debug/else-bot"

# ClientConfig otherwise defaults to the production port. Keep every surface on
# the isolated foreground daemon, even when the developer has a global override.
export ELSE_RUNTIME_URL="http://$BIND"

usage() {
    cat <<'EOF'
usage: scripts/dev.sh <command> [arguments]

Commands:
  serve       Build and run the isolated development daemon in the foreground
  tui         Build and open the TUI against the development runtime
  cli ...     Build and run a CLI command against the development runtime
  seed        Create repeatable decisions, results, and waiting work
  reset       Delete the development runtime after checking it is not running
  check       Run formatting, lint, and workspace tests

Environment:
  ELSE_DEV_RUNTIME_DIR  Runtime directory (default: .else/dev in this repository)
  ELSE_DEV_BIND         Daemon listen address (default: 127.0.0.1:4181)
EOF
}

die() {
    echo "dev: $*" >&2
    exit 1
}

build() {
    cd "$ROOT"
    cargo build -p else-bot
}

require_python() {
    command -v python3 >/dev/null 2>&1 || die "python3 is required for $1"
}

runtime_is_running() {
    ps -axo args= | python3 -c '
import shlex
import sys

runtime, binary = sys.argv[1:]
for line in sys.stdin:
    try:
        arguments = shlex.split(line)
    except ValueError:
        continue
    if (
        arguments
        and arguments[0] == binary
        and "serve" in arguments
        and "--dir" in arguments
        and runtime in arguments
    ):
        raise SystemExit(0)
raise SystemExit(1)
' "$RUNTIME_DIR" "$BINARY"
}

reset_runtime() {
    require_python reset
    local resolved_runtime
    resolved_runtime="$(python3 -c 'from pathlib import Path; import sys; print(Path(sys.argv[1]).resolve())' "$RUNTIME_DIR")"
    case "$resolved_runtime" in
        "$ROOT"/.else/*) ;;
        *) die "reset only allows a runtime inside $ROOT/.else; got $resolved_runtime" ;;
    esac
    [[ "$resolved_runtime" != "$ROOT/.else" ]] || die "refusing to remove the entire .else directory"
    if runtime_is_running; then
        die "development daemon is still running; stop it before reset"
    fi
    rm -rf -- "$resolved_runtime"
    echo "development runtime removed: $resolved_runtime"
}

command_name="${1:-}"
if [[ -z "$command_name" ]]; then
    usage >&2
    exit 2
fi
shift

case "$command_name" in
    serve)
        [[ $# -eq 0 ]] || die "serve takes no arguments"
        build
        echo "development runtime: $RUNTIME_DIR"
        echo "development daemon:  http://$BIND"
        exec "$BINARY" --dir "$RUNTIME_DIR" serve --bind "$BIND"
        ;;
    tui)
        [[ $# -eq 0 ]] || die "tui takes no arguments"
        build
        exec "$BINARY" --dir "$RUNTIME_DIR"
        ;;
    cli)
        build
        exec "$BINARY" --dir "$RUNTIME_DIR" "$@"
        ;;
    seed)
        [[ $# -eq 0 ]] || die "seed takes no arguments"
        require_python seed
        build
        exec python3 "$ROOT/scripts/dev_seed.py" "$BINARY" "$RUNTIME_DIR"
        ;;
    reset)
        [[ $# -eq 0 ]] || die "reset takes no arguments"
        reset_runtime
        ;;
    check)
        [[ $# -eq 0 ]] || die "check takes no arguments"
        cd "$ROOT"
        cargo fmt --all --check
        # 工作流 YAML 写坏了，GitHub 不会报错 —— 它只是把触发器丢掉，
        # 工作流出现在列表里但永远派发不动。所以在本地就解析一遍。
        if command -v ruby >/dev/null 2>&1; then
            for workflow in .github/workflows/*.yml; do
                ruby -ryaml -e 'YAML.load_file(ARGV[0])' "$workflow"
            done
        fi
        cargo clippy --workspace --all-targets -- -D warnings
        cargo test --workspace
        ;;
    help|-h|--help)
        [[ $# -eq 0 ]] || die "help takes no arguments"
        usage
        ;;
    *)
        usage >&2
        die "unknown command: $command_name"
        ;;
esac
