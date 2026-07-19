# Rust quickstart

Add VecLite to your crate:

```toml
[dependencies]
veclite = "0.1"          # crates.io package: hivellm-veclite; lib name: veclite
serde_json = "1"
```

The program below opens a durable single-file database, does a filtered k-NN
search over bring-your-own vectors, and a text search over an offline BM25
auto-embed collection. It is the exact file `cargo xtask docs` runs, so it
cannot drift from the API.

```rust
{{#include ../../../crates/veclite/examples/quickstart.rs}}
```

Run it:

```bash
cargo run -p hivellm-veclite --example quickstart
# veclite 0.1.1: quickstart OK (["a", "c"])
```

The full Rust API is the source of truth for every binding — see
[SPEC-004](../../specs/SPEC-004-rust-api.md).
