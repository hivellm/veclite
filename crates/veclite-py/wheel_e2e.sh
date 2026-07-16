#!/usr/bin/env bash
# Fresh-venv wheel end-to-end (SPEC-009 acceptance 4, phase4h TST-2.3).
#
# Builds the abi3 wheel with maturin, installs it into a throwaway virtualenv
# (proving `pip install veclite` needs no Rust toolchain — PY-001), then runs the
# quickstart and an asyncio `veclite.aio` smoke against the installed package.
#
#   crates/veclite-py/wheel_e2e.sh
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
PYBIN="${PYTHON:-python}"
WHEELS="$HERE/target/wheels"
VENV="$(mktemp -d)/venv"

echo "[wheel-e2e] building abi3 wheel"
( cd "$HERE" && maturin build --release --out "$WHEELS" )

wheel="$(ls -t "$WHEELS"/veclite-*.whl | head -n1)"
echo "[wheel-e2e] wheel: $wheel"

echo "[wheel-e2e] fresh venv: $VENV"
"$PYBIN" -m venv "$VENV"
# venv layout differs by OS.
if [ -x "$VENV/bin/python" ]; then
    VPY="$VENV/bin/python"
else
    VPY="$VENV/Scripts/python.exe"
fi

"$VPY" -m pip install --quiet --upgrade pip
"$VPY" -m pip install --quiet numpy
# Install ONLY the wheel (no build isolation, no sdist) — a compile here means
# the abi3 wheel didn't satisfy the interpreter, which is a failure.
"$VPY" -m pip install --quiet --no-index --no-build-isolation "$wheel"

echo "[wheel-e2e] quickstart + aio smoke"
"$VPY" - <<'PY'
import asyncio
import veclite

# Sync quickstart.
db = veclite.Database.memory()
docs = db.create_collection("docs", dimension=3, metric="euclidean", quantization_bits=0)
docs.upsert("a", [1.0, 0.0, 0.0], {"lang": "en"})
docs.upsert("b", [0.0, 1.0, 0.0])
hits = docs.search([0.9, 0.1, 0.0], limit=1)
assert hits[0]["id"] == "a", hits
assert hits[0]["payload"] == {"lang": "en"}

# Async facade smoke.
async def main():
    adb = veclite.aio.memory()
    ac = await adb.create_collection("d", dimension=3)
    await ac.upsert("p", [1.0, 0.0, 0.0])
    r = await ac.search([1.0, 0.0, 0.0], limit=1)
    assert r[0]["id"] == "p", r

asyncio.run(main())
print("wheel-e2e OK: veclite", veclite.__version__, "abi3 wheel, no toolchain")
PY
echo "[wheel-e2e] PASS"
