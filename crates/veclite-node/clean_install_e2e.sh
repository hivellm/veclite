#!/usr/bin/env bash
# Clean-machine npm install e2e (SPEC-010 acceptance 2, phase4i TST-2.3).
#
# Proves the prebuild install path with NO Rust toolchain: pack the main package
# (loader + types, no .node) plus the current platform's prebuild package (the
# .node), install both into a throwaway project, and run the quickstart. This is
# the optionalDependencies model (NODE-001) — npm resolves the right
# @platform package by os/cpu and the loader requires it.
#
#   crates/veclite-node/clean_install_e2e.sh
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
cd "$HERE"

# 1) Build the prebuild for this host.
echo "[node-e2e] building prebuild"
npx napi build --platform --release >/dev/null

# 2) Identify the built .node and its platform package dir.
node_file="$(ls veclite.*.node | head -n1)"
platform="${node_file#veclite.}"
platform="${platform%.node}"
pkgdir="npm/$platform"
if [ ! -d "$pkgdir" ]; then
    echo "[node-e2e] no platform package dir $pkgdir (run: npx napi create-npm-dir -t .)" >&2
    exit 1
fi
echo "[node-e2e] platform: $platform"

# 3) Stage the .node into its platform package and pack both.
cp "$node_file" "$pkgdir/"
out="$(mktemp -d)"
main_tgz="$(cd "$HERE" && npm pack --silent --pack-destination "$out")"
plat_tgz="$(cd "$pkgdir" && npm pack --silent --pack-destination "$out")"
echo "[node-e2e] packed $main_tgz + $plat_tgz"

# 4) Fresh project: install ONLY the tarballs (no registry, no build).
app="$(mktemp -d)"
cd "$app"
npm init -y >/dev/null 2>&1
npm install --no-audit --no-fund --silent "$out/$plat_tgz" "$out/$main_tgz"

cat > quickstart.mjs <<'JS'
import pkg from '@hivehub/veclite';
const { memory } = pkg;
const db = memory();
const c = await db.createCollection('docs', { dimension: 3, metric: 'euclidean', quantizationBits: 0 });
await c.upsert('a', new Float32Array([1, 0, 0]), { lang: 'en' });
await c.upsert('b', new Float32Array([0, 1, 0]));
const hits = await c.search(new Float32Array([0.9, 0.1, 0]), { limit: 1 });
if (hits[0].id !== 'a' || hits[0].payload.lang !== 'en') {
  throw new Error('unexpected: ' + JSON.stringify(hits));
}
console.log('[node-e2e] quickstart OK from installed prebuild:', hits[0].id);
JS

node quickstart.mjs
echo "[node-e2e] PASS — installed prebuild, no toolchain"
