# SPEC-016 — Packaging, CI & Release Engineering

| | |
|---|---|
| **Status** | Draft |
| **Phase / tasks** | T0.1, T4.6, T6.3, T6.5 ([DAG](../DAG.md)) |
| **PRD requirements** | FR-66; NFR-06, NFR-11, NFR-12; PRD §9 release criteria |
| **Planning source** | [06-sdk-bindings.md §packaging](../vectorizer-lite/06-sdk-bindings.md), [08-roadmap §versioning](../vectorizer-lite/08-roadmap.md) |

Requirement IDs `REL-xxx`.

## 1. Workspace & repository

- **REL-001** Repo `hivellm/veclite`, layout per [02-architecture §repository layout](../vectorizer-lite/02-architecture.md): `crates/{veclite, veclite-ffi, veclite-py, veclite-node, veclite-wasm, veclite-cli}`, `bindings/{go, csharp}`, `docs/`, `tests/`.
- **REL-002** MSRV (resolves OQ-2): VecLite pins its **own** MSRV in `Cargo.toml` (`rust-version`, currently **1.88**), tested in CI; bumping MSRV is a minor-version event, announced in the changelog. The floor is set by the dependency graph, not the edition: edition 2024 floors at 1.85, the pinned `hnsw_rs =0.3.4` raises it to 1.87 (`is_multiple_of`), and `zip` — pulled in by `vecdb-interop`, which `veclite-cli` enables — declares 1.88 across its whole 8.x line. (Originally "track vectorizer-core"; superseded by ADR-0001.)
- **REL-003** Conventional Commits (`feat`/`fix`/`perf`/…); changelog generated per release. No git submodules; **no dependency on any Vectorizer crate via crates.io, git, or path** — needed code is vendored (ADR-0001, CORE-001).

## 2. CI matrix

- **REL-010** Every PR: fmt + clippy `-D warnings` + unit/property tests on {Linux, macOS, Windows} × stable Rust + MSRV; `wasm32-unknown-unknown` build check; dependency deny-list check (no network crates in the default build — NFR-08); footprint check (NFR-06: default-build rlib overhead < 10 MB, clean compile < 60 s on the reference runner).
- **REL-011** Nightly: crash suite (TST-010–013), full-size benches (TST-041), sanitizer runs.
- **REL-012** Release workflow (one reusable GH Actions workflow — PRD risk mitigation): builds the full artifact matrix below, runs binding conformance on each artifact, then publishes everything atomically (all-or-nothing; a failed leg aborts the release).

## 3. Artifact matrix

| Channel | Artifact | Platforms |
|---|---|---|
| crates.io | `veclite`, `veclite-ffi`, `veclite-cli` (source) | n/a |
| PyPI | abi3 wheels (maturin) | manylinux + musllinux + macOS + Windows × x64/arm64 |
| npm | `veclite` + per-platform `@veclite/*` optionalDependencies (napi-rs) | same matrix |
| npm | `@veclite/wasm` | wasm32 (simd128 + fallback) |
| Go | `veclite-go` tag + bundled static libs | same matrix |
| NuGet | `VecLite` with `runtimes/<rid>/native/` | same matrix (RIDs per SPEC-011) |
| GitHub releases | `libveclite.{a,so,dylib}`/`veclite.dll` + `veclite.h`, CLI binaries | same matrix |
| Heavy ONNX | `veclite-onnx` wheel, `@veclite/onnx`, `VecLite.Onnx`, Go tag artifact | same matrix minus wasm |

- **REL-020** **The no-toolchain bar**: installing any package on a clean machine never requires a Rust toolchain (FR-66). Enforced by clean-container/VM install jobs per ecosystem in the release workflow (gate G4/G5).
- **REL-021** Base artifacts never depend on ONNX artifacts; heavy packages declare a dependency on the exact-version base package.

## 4. Versioning & compatibility policy

- **REL-030** SemVer. 0.x during phases 0–5; **1.0.0 at G6**. Core and every binding release in lockstep with one version number (NFR-12).
- **REL-031** File-format version is independent of crate version; format v1 spans many releases. `min_reader_version` gates forward compat (STG header). **Stability pledge published at 1.0**: every future 1.x reads v1 files (NFR-11), backed by the golden-file corpus in CI (SPEC-002 §9.6).
- **REL-032** FFI ABI: additive-only within a major; `vl_abi_version()` gates loaders (FFI-007). Public Rust API: additive-only post-freeze (API-061), enforced by a `cargo public-api` snapshot check.
- **REL-033** Vendored engine code (ADR-0001): any change to shared math/encodings is manually ported between the repos and requires a conformance re-run on both sides before release.

## 5. Documentation deliverables (T6.3)

- **REL-040** Docs site with: quickstarts for all 5 languages + Rust (**each CI-executed** — PRD §9.8), API reference per language, storage-format document (frozen SPEC-002), migration guides both directions (graduation + reverse), benchmark report (TST-042), sizing/limits page, WASM sizing guidance (WASM-012). — Implemented (phase6b): mdBook site at `book.toml` + `docs/src/` (quickstarts ×6 including WASM, guides ×3, reference: limits / storage-format / versioning / benchmarks, specs index); the normative specs and planning docs are linked, not duplicated.
- **REL-041** Every code sample in the docs is extracted and run in CI (doctest or sample-runner) — stale samples fail the build. — Implemented via `cargo xtask docs`: each quickstart page `{{#include}}`s a real runnable file (`crates/veclite/examples/quickstart.rs`, `examples/quickstart.{py,mjs}`, `bindings/go/examples/quickstart/main.go`, `bindings/csharp/Quickstart/Program.cs`, `crates/veclite-wasm/examples/quickstart.mjs`), the runner executes each (probing the toolchain, skipping — never silently passing — when it is absent), plus a relative-link checker over all docs and an `mdbook build`. Local-first (Actions are off): the full six-language matrix runs where the packages are installed; a machine runs what it has.

## 6. Release checklist (1.0.0 — mirrors PRD §9)

1. All DAG gates G0–G5 green; G6 criteria checked item-by-item in the release PR description.
2. Crash suite 10 000-iteration run within the last 7 days on all 3 OS.
3. Conformance corpus green: Rust, Python, Node, Go, C#, WASM × full platform matrix.
4. Graduation round-trip vs the pinned server version ≥ 0.99 overlap.
5. Fuzz 72 h clean; 24 h soak clean; sanitizers clean.
6. Footprint + compile-time budgets green.
7. Format stability pledge + SemVer policy published on the docs site.
8. Clean-machine install verification for every ecosystem.
9. Tag → atomic multi-channel publish (REL-012) → post-publish smoke installs.

## 7. Acceptance criteria

1. REL-010 pipeline exists from T0.1 (bootstrap) and stays green throughout.
2. Release dry-run (publish to test indexes: TestPyPI, npm dist-tag `next`, NuGet test feed) executed at G4 and G5 before the real 1.0.0.
3. The 1.0.0 checklist above is a PR template in the repo (`.github/RELEASE_TEMPLATE.md`).
