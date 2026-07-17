#!/usr/bin/env bash
# Build the @veclite/wasm package: two wasm binaries (simd128 + a plain
# fallback, WASM-002) that share one wasm-bindgen glue, plus the size report the
# CI budget gate consumes (WASM-030). Requires `wasm-bindgen` (version-matched to
# the `wasm-bindgen` crate in Cargo.toml) and, optionally, `wasm-opt` for the
# final size squeeze.
#
#   ./build-pkg.sh          # build pkg/
#   ./build-pkg.sh --check  # build, then fail if any wasm exceeds the 3 MB gzip budget
set -euo pipefail
cd "$(dirname "$0")"

OUT="pkg"
TARGET_WASM="target/wasm32-unknown-unknown/release/veclite_wasm.wasm"
BUDGET=$((3 * 1024 * 1024)) # 3 MB gzipped (WASM-030)

rm -rf "$OUT"
mkdir -p "$OUT" .build-tmp

echo "[1/4] fallback build (no simd128)"
cargo build --target wasm32-unknown-unknown --release
cp "$TARGET_WASM" .build-tmp/fallback.wasm

echo "[2/4] simd128 build (-C target-feature=+simd128)"
RUSTFLAGS="-C target-feature=+simd128" cargo build --target wasm32-unknown-unknown --release
cp "$TARGET_WASM" .build-tmp/simd.wasm

echo "[3/4] wasm-bindgen glue (target web; one glue, two wasm)"
# The bindgen interface is identical for both builds (same crate, same version);
# simd128 only changes internal codegen. Generate the glue once from the
# fallback, then bring in the simd binary alongside it.
wasm-bindgen --target web --out-dir "$OUT" --out-name veclite_core .build-tmp/fallback.wasm
mv "$OUT/veclite_core_bg.wasm" "$OUT/veclite_fallback.wasm"
wasm-bindgen --target web --out-dir .build-tmp/simd_pkg --out-name veclite_core .build-tmp/simd.wasm
cp .build-tmp/simd_pkg/veclite_core_bg.wasm "$OUT/veclite_simd.wasm"

echo "[4/4] wasm-opt -Oz (if available)"
if command -v wasm-opt >/dev/null 2>&1; then
  for w in "$OUT/veclite_fallback.wasm" "$OUT/veclite_simd.wasm"; do
    wasm-opt -Oz --enable-simd "$w" -o "$w"
  done
else
  echo "  wasm-opt not found — skipping (binaries already within budget)"
fi

rm -rf .build-tmp

echo
echo "size report (gzipped budget: $((BUDGET / 1024 / 1024)) MB):"
fail=0
for w in "$OUT/veclite_fallback.wasm" "$OUT/veclite_simd.wasm"; do
  raw=$(wc -c <"$w")
  gz=$(gzip -c "$w" | wc -c)
  printf "  %-28s raw %8d  gzip %8d\n" "$(basename "$w")" "$raw" "$gz"
  if [ "$gz" -gt "$BUDGET" ]; then
    echo "  ::error:: $(basename "$w") gzip $gz exceeds budget $BUDGET (WASM-030)"
    fail=1
  fi
done

if [ "${1:-}" = "--check" ] && [ "$fail" -ne 0 ]; then
  exit 1
fi
echo "done -> $OUT/"
