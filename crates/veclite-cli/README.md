# hivellm-veclite-cli

Command-line tool for [`.veclite`](https://github.com/hivellm/veclite) databases:
inspect, verify, vacuum, snapshot, and `.vecdb` interop with a Vectorizer server.

Installs a binary named **`veclite`**.

```sh
cargo install hivellm-veclite-cli
```

## Commands

| Command | What it does |
|---|---|
| `inspect` | Header, format version, sizes, per-collection configuration and segment breakdown. Opens read-only (shared lock). |
| `verify` | Read-only integrity pass: header, TOC, every segment CRC and body, collection reconstruction, WAL scan. |
| `vacuum` | Reclaim dead space in place. |
| `snapshot` | Write a compacted, standalone point-in-time copy. |
| `export` | Export collections to a Vectorizer server data directory (`vectorizer.vecdb` + `vectorizer.vecidx`, Compact layout). |
| `import` | Import a Vectorizer data set (Compact `.vecdb` or Legacy `*_vector_store.bin`) into a new `.veclite` database. |

```sh
veclite inspect data.veclite
veclite verify data.veclite
veclite snapshot data.veclite backup.veclite
```

`verify` is scriptable: it exits `0` when the file is clean and `1` when
corruption is found, printing every finding with its segment offset and type.

## Related crates

- [`hivellm-veclite`](https://crates.io/crates/hivellm-veclite) — the embeddable library
- [`hivellm-veclite-ffi`](https://crates.io/crates/hivellm-veclite-ffi) — C ABI

## License

Apache-2.0
