#!/usr/bin/env bash
# Build and run the C smoke programs under the sanitizers (SPEC-008 phase4g
# TST-2.1/2.2). Linux/clang only — ASan+LSan and TSan need the Rust library
# instrumented, so this is a CI (Linux) gate, not a Windows-dev check.
#
#   tests/c/sanitize.sh            # run every available check
#   tests/c/sanitize.sh asan       # AddressSanitizer + LeakSanitizer only
#   tests/c/sanitize.sh tsan       # ThreadSanitizer only
#   tests/c/sanitize.sh valgrind   # Valgrind leak-check (no instrumentation)
#
# ASan/LSan: full_smoke.c linked against the release staticlib; LeakSanitizer
# intercepts malloc globally, so a leak of any handle/buf that reaches the system
# allocator fails the run. TSan: concurrency.c linked against a TSan-instrumented
# build of the Rust library (nightly + -Zbuild-std) — any data race inside the
# library fails the run.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
HDR_DIR="$ROOT/crates/veclite-ffi"
CDIR="$ROOT/tests/c"
TARGET="${SANITIZE_TARGET:-x86_64-unknown-linux-gnu}"
CC="${CC:-clang}"
LINK_LIBS="-lpthread -ldl -lm"
WHICH="${1:-all}"

[ "$(uname -s)" = "Linux" ] || {
    echo "sanitize.sh: Linux/clang only (got $(uname -s)); skipping" >&2
    exit 0
}

# Ensure the committed header is in sync before we trust it.
run_asan() {
    echo "[sanitize] ASan + LSan: full_smoke"
    cargo build -p hivellm-veclite-ffi --release --target "$TARGET" >/dev/null
    local lib="$ROOT/target/$TARGET/release/libveclite_ffi.a"
    local bin
    bin="$(mktemp -d)/full_smoke"
    "$CC" -std=c11 -g -O1 -fsanitize=address -fno-omit-frame-pointer \
        "$CDIR/full_smoke.c" -I"$HDR_DIR" "$lib" $LINK_LIBS -o "$bin"
    ASAN_OPTIONS="detect_leaks=1:halt_on_error=1" "$bin"
    echo "[sanitize] ASan + LSan OK"
}

run_tsan() {
    echo "[sanitize] TSan: concurrency (instrumented Rust lib)"
    RUSTFLAGS="-Zsanitizer=thread" cargo +nightly build -Zbuild-std \
        --release --target "$TARGET" -p hivellm-veclite-ffi >/dev/null
    local lib="$ROOT/target/$TARGET/release/libveclite_ffi.a"
    local bin
    bin="$(mktemp -d)/concurrency"
    "$CC" -std=c11 -g -O1 -fsanitize=thread -fno-omit-frame-pointer \
        "$CDIR/concurrency.c" -I"$HDR_DIR" "$lib" $LINK_LIBS -o "$bin"
    TSAN_OPTIONS="halt_on_error=1" "$bin"
    echo "[sanitize] TSan OK"
}

run_valgrind() {
    command -v valgrind >/dev/null || {
        echo "[sanitize] valgrind not installed; skipping" >&2
        return 0
    }
    echo "[sanitize] Valgrind memcheck: full_smoke (uninstrumented)"
    cargo build -p hivellm-veclite-ffi --release --target "$TARGET" >/dev/null
    local lib="$ROOT/target/$TARGET/release/libveclite_ffi.a"
    local bin
    bin="$(mktemp -d)/full_smoke_plain"
    "$CC" -std=c11 -g -O1 "$CDIR/full_smoke.c" -I"$HDR_DIR" "$lib" $LINK_LIBS -o "$bin"
    valgrind --leak-check=full --errors-for-leak-kinds=definite \
        --error-exitcode=1 "$bin"
    echo "[sanitize] Valgrind OK"
}

case "$WHICH" in
    asan) run_asan ;;
    tsan) run_tsan ;;
    valgrind) run_valgrind ;;
    all)
        run_asan
        run_tsan
        ;;
    *)
        echo "usage: sanitize.sh [asan|tsan|valgrind|all]" >&2
        exit 2
        ;;
esac
echo "[sanitize] PASS"
