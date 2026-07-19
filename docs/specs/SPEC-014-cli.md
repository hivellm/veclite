# SPEC-014 — `veclite` CLI

| | |
|---|---|
| **Status** | Implemented (phase5d) — resolves PRD OQ-4: the CLI is a **separate `veclite-cli` crate** in the workspace, installing a binary named `veclite` (keeps the library dependency tree clean; the CLI enables `vecdb-interop`). `--help` output is snapshot-tested (`crates/veclite-cli/tests/snapshots/`) |
| **Phase / tasks** | Phase 5 · T5.5 ([DAG](../DAG.md)) |
| **PRD requirements** | FR-71 |
| **Planning source** | [07-vectorizer-compatibility.md §graduation](../vectorizer-lite/07-vectorizer-compatibility.md) |

Requirement IDs `CLI-xxx`. The CLI is a thin veneer over the library + SPEC-013 interop; it adds no engine behavior of its own.

## 1. Commands (v1)

```
veclite inspect <db.veclite> [--json]
veclite export  <db.veclite> --format vecdb --out <dir> [--collections a,b]
veclite import  <src.vecdb|src-dir> --out <db.veclite> [--collections a,b] [--force]
veclite vacuum  <db.veclite>
veclite snapshot <db.veclite> --out <copy.veclite>
veclite verify  <db.veclite>
```

| Command | Behavior |
|---|---|
| `inspect` | Header, format version, file/WAL size, per-collection: config, vector count, tombstones, segment breakdown. `--json` for machine consumption. Opens read-only (shared lock). |
| `export` | SPEC-013 §2 graduation export. Prints a summary (collections, vectors, bytes) and any warnings. |
| `import` | SPEC-013 §3 reverse path. Refuses to overwrite an existing output unless `--force`. Prints the degradation warnings table. |
| `vacuum` / `snapshot` | Direct library calls (FR-05/06) for scripting. |
| `verify` | Read-only integrity pass: header + TOC + every segment crc, WAL scan, IDDIR consistency. Exit 0 = clean; 1 = corruption found (each finding printed with segment offset/type). |

- **CLI-001** Exit codes: 0 success · 1 data/integrity error · 2 usage error · 3 environment error (`Locked`, permissions, disk full). Stable — scripts depend on them.
- **CLI-002** All commands honoring locks: mutating commands take the exclusive lock and fail fast with a clear message when `Locked`; `inspect`/`verify` open read-only.
- **CLI-003** Output is human-readable by default; `--json` (where offered) emits a stable, documented schema. Warnings go to stderr; data to stdout.
- **CLI-004** No network access, matching NFR-08. Distribution: `cargo install veclite-cli` + prebuilt binaries in GitHub releases (FR-66 matrix).

## 2. Acceptance criteria

1. Round-trip smoke in CI: `import` → `inspect --json` → `export` → server-side verification fixture (with SPEC-013 tests).
2. `verify` detects each bit-flip drill fixture from SPEC-002 §9.3 with the correct segment identification.
3. Exit-code contract covered by integration tests.
4. `--help` for every command generated and snapshot-tested (docs stay in sync).
