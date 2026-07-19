# Versioning & format stability

VecLite has **two** version numbers that move independently.

## Crate / package version (SemVer)

The library and every binding release in lockstep under one SemVer number
(NFR-12). During phases 0–5 the version is 0.x; **1.0.0 ships at gate G6**. After
1.0:

- **Public Rust API** is additive-only within a major version, enforced by a
  committed `cargo public-api` snapshot (`cargo xtask api-freeze`).
- **C ABI** is additive-only within a major; `vl_abi_version()` gates loaders.
- **MSRV** (minimum supported Rust version, currently **1.87**) is tested in CI;
  a bump is a minor-version event announced in the changelog.

## File-format version

The `.veclite` format version is **independent** of the crate version — format
v1 spans many releases. The 4 KiB header carries a `min_reader_version`; a file
that demands a newer reader than the running build fails cleanly with
`UnsupportedFormatVersion` rather than misreading.

### The stability pledge (published at 1.0)

> **Every future 1.x release reads `.veclite` v1 files.**

Files written today stay readable. The pledge is backed by a committed
golden-file corpus in CI: a v1 file created by an early build is opened and
verified by every later build, so a format regression fails the build. The
byte-format is **frozen-normative** (SPEC-002) — changing it is a major-version,
new-format-version event, never a silent break.

There is no automated downgrade: a newer build can always read an older v1 file,
but rolling the binary back after a hypothetical future format bump may require
restoring a pre-upgrade backup. Within v1 this never arises.

See [SPEC-016 §4](../../specs/SPEC-016-packaging-release.md) for the full
versioning and compatibility policy, and [SPEC-002
§9](../../specs/SPEC-002-storage-format.md) for the golden-file machinery.
