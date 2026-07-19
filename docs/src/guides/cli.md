# The `veclite` CLI

A single binary to inspect, verify, and maintain `.veclite` databases and to
exchange data with a Vectorizer server. It is a thin veneer over the library —
no engine behavior of its own, no network access, ever.

```bash
cargo install hivellm-veclite-cli     # installs a binary named `veclite`
```

```
Usage: veclite <COMMAND>

Commands:
  inspect   Header, format version, sizes, per-collection config + segments (read-only)
  export    Export collections to a Vectorizer server data set (.vecdb + .vecidx)
  import    Import a Vectorizer server data set into a new .veclite database
  vacuum    Reclaim dead space in place
  snapshot  Write a compacted, standalone point-in-time copy
  verify    Read-only full-file integrity pass
```

## Exit codes (stable — scripts depend on them)

| Code | Meaning |
|---|---|
| 0 | success |
| 1 | data / integrity error (corruption found) |
| 2 | usage error (bad arguments) |
| 3 | environment error (`Locked`, permissions, disk full) |

Warnings go to stderr, data to stdout; `--json` (where offered) emits a stable
schema. Mutating commands take the exclusive advisory lock and fail fast with
exit 3 when the file is already open read-write; `inspect` and `verify` open
read-only.

## Common tasks

**Inspect** a database (read-only) — header, generation, per-collection config
and segment breakdown:

```bash
veclite inspect app.veclite            # human-readable
veclite inspect app.veclite --json     # stable machine schema
```

**Verify** integrity — header, TOC, every segment CRC and body, collection
reconstruction, and the WAL scan. Exit 0 = clean; exit 1 names each damaged
segment by offset and type:

```bash
veclite verify app.veclite
```

**Reclaim space** after bulk deletes (tombstones), in place:

```bash
veclite vacuum app.veclite
```

**Snapshot** — a compacted, standalone point-in-time copy that opens
independently:

```bash
veclite snapshot app.veclite --out backup.veclite
```

**Export / import** — the graduation and reverse paths; see the
[graduation](graduation.md) and [reverse-migration](reverse-migration.md)
guides:

```bash
veclite export app.veclite --format vecdb --out ./export/
veclite import ./data/vectorizer.vecdb --collections docs,notes --out app.veclite
```

The full contract is [SPEC-014](../../specs/SPEC-014-cli.md); every command's
`--help` output is snapshot-tested so the docs stay in sync with the binary.
