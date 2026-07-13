# SPEC-003 — WAL & Durability

| | |
|---|---|
| **Status** | Draft — frozen with format v1 at gate G2 |
| **Phase / tasks** | Phase 2 · T2.3, T2.4 ([DAG](../DAG.md)) |
| **PRD requirements** | FR-07, FR-51, FR-52; NFR-05, NFR-07 |
| **Planning source** | [04-storage-format.md §write path](../vectorizer-lite/04-storage-format.md) |

Requirement IDs `WAL-xxx`. Integers little-endian. The main-file commit protocol is SPEC-002 §5.

## 1. Model

Write-ahead logging with in-memory apply:

1. Every mutating API call appends **one WAL entry** (a batch is one entry — the atomic unit).
2. In-memory state (HNSW, payload maps, vocab) updates immediately after the append succeeds — readers in the same process see the write at once.
3. **Checkpoint** transfers accumulated state into sealed main-file segments (SPEC-002 §5), then truncates the WAL.
4. **Recovery** replays the WAL into memory on open when the previous close wasn't clean.

## 2. WAL file

- **WAL-001** Sidecar file `<db>.veclite-wal` in the same directory (SQLite naming convention). Created lazily on first write; deleted (or truncated to the 16-byte WAL header) on clean close after checkpoint.
- **WAL-002** WAL header (16 bytes): magic `VLWL` (4) · `format_version u32` (=1) · `file_uuid_prefix [8]` = first 8 bytes of the main file's UUID. On open, a WAL whose uuid prefix does not match the main file MUST be ignored and reported via the warning callback (stale sidecar from a copied database).
- **WAL-003** In-memory databases (`VecLite::memory()`) have no WAL and no durability; all `Durability` settings are no-ops there.

## 3. Entry format

```
| seq u64 | coll_id u32 | op u8 | reserved u8[3] | body_len u32 | body_crc32 u32 | body (MessagePack) |
```

| `op` | Name | Body |
|---|---|---|
| 1 | `UPSERT_BATCH` | `[Point]` (id, vector or text, sparse?, payload?) |
| 2 | `DELETE_BATCH` | `[id]` |
| 3 | `CREATE_COLL` | full `CollectionConfig` + assigned `coll_id` |
| 4 | `DROP_COLL` | — |
| 5 | `RENAME` | `{ new_name }` |
| 6 | `ALIAS` | `{ action: create|delete, alias, target }` |
| 7 | `VOCAB_UPDATE` | provider state delta or full snapshot (SPEC-005 §5) |
| 8 | `PIDX_DECLARE` | `{ key, kind }` (late-added payload index) |

- **WAL-010** `seq` starts at 1 after each checkpoint and increases by 1 per entry. A gap or non-monotonic `seq` during replay MUST stop replay at the last contiguous entry.
- **WAL-011** `body_crc32` covers the body bytes. An entry with a bad crc terminates replay: it and everything after it are discarded (torn tail). Entries **before** it are kept — a mid-file crc failure with valid entries after it is impossible under append-only writing and MUST be treated as the torn tail (discard from the bad entry onward).
- **WAL-012** The whole entry is the atomic unit: a partially applied batch MUST never be observable, in memory or after recovery.

## 4. Durability modes

`OpenOptions::durability(Durability)` — default `Normal`:

| Mode | fsync on WAL append | fsync at checkpoint/close | Guarantee after OS crash |
|---|---|---|---|
| `Full` | every entry | yes | every acked write is durable |
| `Normal` (default) | no | yes | writes since the last checkpoint may be lost; file never corrupt |
| `Off` | no | no (checkpoint still ordered-writes) | any un-checkpointed data may be lost; file never corrupt |

- **WAL-020** "Never corrupt" holds in **all** modes: durability tuning trades freshness, not integrity (STG-003 + WAL-011 guarantee this).
- **WAL-021** Process-crash (not OS-crash) guarantee: in every mode, entries fully written to the OS page cache survive; the crash suite (kill-9) MUST therefore pass with zero lost acked writes in `Full` and zero corruption in all modes.

## 5. Checkpoint

- **WAL-030** Triggers: (a) WAL size ≥ threshold (default 64 MiB, `OpenOptions::wal_size_limit`); (b) explicit `db.checkpoint()`; (c) clean close (last handle dropped); (d) auto-vacuum escalation (STG-072). With `background_checkpoint(true)` a helper thread MAY run (a) opportunistically; otherwise checkpoints run on the calling thread of the write that crossed the threshold.
- **WAL-031** Sequence: seal in-memory deltas into new segments → SPEC-002 §5 commit protocol → truncate WAL to its 16-byte header → reset `seq`. Readers proceed throughout except the TOC-swap instant (STG-061).
- **WAL-032** A crash **during** checkpoint recovers to the pre-checkpoint state (old TOC + full WAL) or the post-checkpoint state (new TOC + empty WAL) — never in between. The WAL MUST be truncated only **after** the header swap fsync completes.

## 6. Recovery (on read-write open)

- **WAL-040** If header `clean_close` = 1 and the WAL is absent/empty → open directly.
- **WAL-041** Otherwise: load main file from TOC; replay WAL entries in `seq` order, applying each atomically to in-memory state; stop at the first invalid entry (WAL-010/011); set `clean_close` = 0 in the running state; the next checkpoint persists the replayed writes.
- **WAL-042** Replay MUST be idempotent with respect to the main file: an entry whose effects are already in the checkpointed state (possible only for the crash-during-checkpoint window where WAL truncation lost the race — prevented by WAL-032 ordering) is a design-impossible state; if detected (e.g., `CREATE_COLL` for an existing coll_id with identical config), replay MUST treat it as a no-op rather than fail.
- **WAL-043** `read_only` open never replays; if a non-empty WAL exists, the reader MUST fail with `Locked`-class error `WalPending` unless `OpenOptions::read_only_ignore_wal(true)` is set (documented as "reads the last checkpoint, ignoring newer writes").

## 7. Close semantics

- **WAL-050** Drop of the last `Database` handle: flush + checkpoint (per durability mode), set `clean_close` = 1 via header rewrite, release the lock. `close()` is idempotent; operations on a closed handle → `Closed` error.
- **WAL-051** If checkpoint-on-close fails (e.g., disk full), the WAL MUST be left intact (recovery will replay) and the error surfaced from `close()`; Drop swallows the error but MUST leave the same recoverable state.

## 8. Acceptance criteria

1. **Replay property tests** (T2.3): arbitrary interleavings of upsert/delete/create/drop/alias/vocab ops → crash at every entry boundary → replayed state ≡ model state.
2. **Torn-tail fuzz**: truncate/corrupt the WAL at every byte offset of the last entry → open succeeds, entries before the tear intact.
3. **kill-9 matrix** (T2.10): all three durability modes × kill points (mid-append, mid-checkpoint each fsync step, mid-truncate) × 10 000 iterations — invariants of WAL-020/021/032 hold.
4. **Stale-WAL test**: copy `db.veclite` without its WAL next to a foreign WAL → ignored with warning (WAL-002).
5. **Checkpoint-under-load**: concurrent readers observe either pre- or post-checkpoint state, never a mix; TOC swap stall < 1 ms p99 on the reference profile.
