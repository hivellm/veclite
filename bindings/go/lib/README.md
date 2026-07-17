# Bundled native libraries

Each `lib/<goos>_<goarch>/` holds the prebuilt VecLite C ABI library the cgo
directive files (`../cgo_<platform>.go`) link:

- Linux / macOS: `libveclite_ffi.a` (static — self-contained binary, GO-001).
- Windows: `veclite_ffi.dll` + `veclite_ffi.lib` (import lib; the dll must be on
  PATH or beside the executable — static linking would have to bundle the
  `windows` crate's import libraries).

These binaries are **not** committed; the release CI builds `veclite-ffi` per
platform and drops the artifact here (mirroring the napi prebuild model). Build
them locally with:

    cargo build -p veclite-ffi --release   # target/release/veclite_ffi.*

then copy the artifact for your platform into the matching `lib/<goos>_<goarch>/`.
The header (`../internal/csrc/veclite.h`) is the frozen C ABI, synced from
`crates/veclite-ffi/veclite.h` (drift-tested there).
