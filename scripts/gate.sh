#!/usr/bin/env bash
# Single entry point for every cargo invocation made by agents and the orchestrator.
#
# usage: scripts/gate.sh [--slot N|main] <mode> [args]
#   crate <crate>...        fmt, clippy, test, doc, typos for the crates + workspace check + golden
#   workspace               mirrors CI: fmt, clippy, test, doc, deny, typos
#   golden                  wire-compatibility tests (golden fixtures, descriptor lock, round trips)
#   live <crate> [filter]   #[ignore]d network tests, one at a time, with a byte budget
#   cargo <args...>         any cargo command with the slot environment
#
# Each slot has its own target directory so parallel worktrees don't block each other on
# cargo's build-directory lock, while warm dependency artifacts are reused across worktrees.
# Output is kept short: one PASS/FAIL line per step; full logs go to $LOG_DIR.

set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

TARGET_BASE="${A3S_TARGET_BASE:-/run/media/system/Main_storage/cargo-target/a3s}"
LOG_DIR="$TARGET_BASE/logs"

SLOT=""
if [[ "${1:-}" == "--slot" ]]; then
    SLOT="$2"
    shift 2
fi
if [[ -z "$SLOT" && -n "${A3S_SLOT:-}" ]]; then SLOT="$A3S_SLOT"; fi
if [[ -z "$SLOT" && -f "$ROOT/.a3s-slot" ]]; then SLOT="$(tr -d '[:space:]' < "$ROOT/.a3s-slot")"; fi
if [[ -z "$SLOT" ]]; then SLOT="main"; fi

MODE="${1:-}"
if [[ -z "$MODE" ]]; then
    sed -n '2,12p' "$0"
    exit 2
fi
shift

mkdir -p "$TARGET_BASE" "$LOG_DIR"
export CARGO_TARGET_DIR="$TARGET_BASE/slot-$SLOT"
export CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}"
export CARGO_TERM_COLOR=never
export RUSTFLAGS="-D warnings"
export RUSTDOCFLAGS="-D warnings"
# JDK for the Java fixture harness (WP01) and #[ignore]d Java cross-check tests.
if [[ -z "${JAVA_HOME:-}" && -f "$ROOT/scripts/java_env.sh" ]]; then
    # shellcheck disable=SC1091
    source "$ROOT/scripts/java_env.sh" >/dev/null 2>&1 || true
fi
# Headless Vulkan for egui_kittest snapshot tests.
export VK_ICD_FILENAMES="${VK_ICD_FILENAMES:-/usr/share/vulkan/icd.d/lvp_icd.x86_64.json}"

STAMP="$(date +%Y%m%d-%H%M%S)"
FAILED=0

# run <step-name> <command...>: runs under the slot lock, prints one line, logs everything.
run() {
    local name="$1"
    shift
    local log="$LOG_DIR/${SLOT}-${STAMP}-${name//[^a-zA-Z0-9_-]/_}.log"
    local start end
    start=$(date +%s)
    flock "$CARGO_TARGET_DIR.lock" "$@" >"$log" 2>&1
    local status=$?
    end=$(date +%s)
    if [[ $status -eq 0 ]]; then
        echo "PASS $name ($((end - start))s)"
    else
        echo "FAIL $name ($((end - start))s) log: $log"
        grep -E -m 80 -A6 '^(error|warning)(\[|:)|^test .* FAILED|panicked|^FAIL|^Diff in|^failures:' "$log" | head -80
        FAILED=1
    fi
    return $status
}

crate_args() {
    local args=()
    for c in "$@"; do args+=(-p "$c"); done
    echo "${args[@]}"
}

crate_dir() {
    cargo metadata --no-deps --format-version 1 2>/dev/null |
        python3 -c "import json,sys,os; m=json.load(sys.stdin); print(next(os.path.dirname(p['manifest_path']) for p in m['packages'] if p['name']==sys.argv[1]))" "$1"
}

golden() {
    run golden cargo test --all-features \
        -p a3s_jserial -p a3s_types -p a3s_repo_files -p a3s_zsync
}

case "$MODE" in
    crate)
        [[ $# -ge 1 ]] || { echo "usage: gate.sh crate <crate>..."; exit 2; }
        # shellcheck disable=SC2046
        PKGS=$(crate_args "$@")
        run fmt cargo fmt --all -- --check
        # shellcheck disable=SC2086
        run clippy cargo clippy $PKGS --all-targets --all-features -- -D warnings
        # shellcheck disable=SC2086
        run test cargo test $PKGS --all-features
        # shellcheck disable=SC2086
        run doc cargo doc $PKGS --no-deps --all-features
        for c in "$@"; do
            dir="$(crate_dir "$c")"
            [[ -n "$dir" ]] && run "typos-$c" typos "$dir"
        done
        run workspace-check cargo check --workspace --all-targets --all-features
        golden
        ;;
    workspace)
        run fmt cargo fmt --all -- --check
        run clippy cargo clippy --workspace --all-targets --all-features -- -D warnings
        run test cargo test --workspace --all-features
        run doc cargo doc --workspace --no-deps --all-features
        run deny cargo deny check
        run typos typos
        ;;
    golden)
        golden
        ;;
    live)
        [[ $# -ge 1 ]] || { echo "usage: gate.sh live <crate> [filter]"; exit 2; }
        CRATE="$1"
        shift
        export A3S_LIVE_MAX_BYTES="${A3S_LIVE_MAX_BYTES:-67108864}"
        run "live-$CRATE" flock "$TARGET_BASE/live.lock" timeout 900 \
            cargo test -p "$CRATE" --all-features -- --ignored --test-threads=1 "$@"
        ;;
    cargo)
        flock "$CARGO_TARGET_DIR.lock" cargo "$@"
        exit $?
        ;;
    *)
        echo "unknown mode: $MODE"
        exit 2
        ;;
esac

exit $FAILED
