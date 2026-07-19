## 1. Implementation
- [x] 1.1 Context: read docs/specs/SPEC-013 and SPEC-014 in full; DAG T5.5, T5.6 and gate G5 — plus the pinned server's storage sources (writer/reader/index/persistence/models/tokenizers) to fix the exact wire shapes
- [x] 1.2 Export writer: Compact .vecdb + .vecidx accepted by the server StorageReader; scope options (IOP-010..013) — `interop::export_vecdb` (feature `vecdb-interop`): ZIP/DEFLATE + SHA-256 .vecidx, f32-exact vectors, `_text`→`content`, per-provider tokenizer translation, aliases/pidx/BYO in .vecidx metadata, refit settles before snapshot
- [x] 1.3 Import reader: detect_format for Compact + Legacy; --collections subsetting (IOP-020/021) — `interop::{detect_layout, import_vecdb}`; Legacy gzip+plain JSON, .vecidx optional-with-warning
- [x] 1.4 Degradation matrix: tenant/shard/graph warnings, encrypted refusal, BYO fallback with origin_provider (IOP-022/023) — owner/tenant + sharding + graph + normalization + seed + PQ/SQ-N warnings; required-encryption refuses before any creation; server-only providers defer to a Missing slot (origin kept in CONFIG)
- [x] 1.5 crates/veclite-cli: inspect/export/import/vacuum/snapshot/verify; exit codes 0/1/2/3; --json where offered (CLI-001..003) — binary `veclite` (OQ-4 resolved: separate crate); warnings→stderr, data→stdout; mutating cmds exclusive lock, inspect/verify read-only; import refuses existing output without --force and cleans up on failure
- [x] 1.6 verify command: full-file integrity pass naming damaged segments (CLI table) — `interop::verify_file`: header → TOC CRC → per-segment frame CRC + body decode → collection reconstruction → read-only WAL scan; findings carry offset + segment type
- [x] 1.7 Graduation round-trip automation vs dockerized pinned server (TST-032) — `cargo xtask graduation`: deterministic standard corpus, committed golden, two export→import cycles, then the server-side test in the pinned Vectorizer repo. NOTE (assumption stated): GitHub Actions are off (quota) and no server docker image is published, so the server check runs against the pinned server *sources* (`crates/vectorizer/tests/veclite_compat.rs`, its own StorageReader + BM25 provider) — the same code a dockerized server would execute, without network
- [x] 1.8 Wire the shared conformance corpus into both repos' CI (IOP-002) — corpus golden committed at `tests/compat/vecdb/golden.json` (VecLite, exercised by `cargo xtask graduation`) and mirrored with the exported fixture at `crates/vectorizer/tests/compat/veclite/` (server repo, plain `cargo test --test veclite_compat`); both run in each repo's local quality gate (Actions disabled)

## 2. Testing
- [x] 2.1 Round-trip: export → server import → top-10 overlap >= 0.99; bm25 scores within 1e-5 — measured overlap 1.0000 (text and vectors) on the server side; BM25 query embeddings reproduced by the server provider within 1e-5 from the exported tokenizer
- [x] 2.2 Reverse round-trip stable on a second cycle (no drift) — cycle-2 overlap 1.0000, scores within 1e-5 (xtask graduation) + unit round-trip in `interop::import` tests
- [x] 2.3 Legacy-layout fixture imports correctly — gzip + plain JSON legacy fixtures in `interop::import` tests (config from sibling metadata, owner field warned)
- [x] 2.4 Degradation fixtures for every matrix row; verify detects every bit-flip fixture — owner/tenant, sharding, graph, normalization, seed, PQ/SQ-N, encrypted refusal, server-only provider BYO fallback (config + import tests); verify bit-flip sweep across every live segment type asserts offset+type (`interop::verify` tests)
- [x] 2.5 CLI exit-code contract integration tests; --help snapshots — `crates/veclite-cli/tests/cli.rs`: exit 0/1/2/3 pinned (clean, corrupt, usage/--force, locked/missing), committed `--help` snapshots for all commands (bin_name pinned for platform-stable output)

## 3. Tail (mandatory — enforced by rulebook v5.3.0)
- [x] 3.1 Update or create documentation covering the implementation — SPEC-013/SPEC-014 status → Implemented; README "Interop & Tooling" section + roadmap row; module docs throughout `interop/`
- [x] 3.2 Write tests covering the new behavior — 26 interop unit tests (model/vocab/config/export/import/verify/inspect), 9 CLI integration tests, graduation gate both repos
- [x] 3.3 Run tests and confirm they pass (gate G5 evidence attached) — see Evidence below

## Evidence (G5)
- `cargo xtask graduation --vectorizer e:\HiveLLM\Vectorizer`: golden PASS; cycle-1 overlap text 1.0000 / vectors 1.0000; cycle-2 stable (1.0000/1.0000); server-side `veclite_compat` PASS ("overlap text 1.0000, vectors 1.0000 (gate 0.99)")
- VecLite commits: 8c3b4d3 (interop core), ee750f7 (verify/inspect), ed69bc0 (CLI), + graduation gate commit; Vectorizer repo: test + fixture committed alongside
