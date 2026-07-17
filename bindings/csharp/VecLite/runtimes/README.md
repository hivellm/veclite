# Bundled native libraries (NuGet runtimes layout, CS-001)

Each `runtimes/<rid>/native/` holds the prebuilt VecLite C ABI shared library
for one RID (`win-x64`, `linux-x64`, `linux-arm64`, `linux-musl-x64`, `osx-x64`,
`osx-arm64`, `win-arm64`). The `Native` P/Invoke resolver loads the file for the
current RID from here.

These binaries are **not** committed; the release CI builds `veclite-ffi` per
platform and drops `veclite_ffi.{dll,so,dylib}` into the matching folder. Build
locally with `cargo build -p veclite-ffi --release` and copy the artifact into
`runtimes/<your-rid>/native/`.
