#!/usr/bin/env bash
# Compile ffi/smoke.c against a shipped VecLite C-ABI bundle and run it, proving
# the prebuilt library links and loads with no Rust toolchain (SPEC-008,
# phase4e; mirrors phase4d's clean-machine bar).
#
# Usage: ffi/smoke.sh <bundle-dir>
# <bundle-dir> holds veclite.h + the shipped library:
#   Linux:   libveclite.so                 macOS: libveclite.dylib
#   Windows: veclite.dll (+ veclite.dll.lib import lib)
set -euo pipefail

BUNDLE="${1:?usage: smoke.sh <bundle-dir>}"
HERE="$(cd "$(dirname "$0")" && pwd)"
SRC="$HERE/smoke.c"
BUNDLE="$(cd "$BUNDLE" && pwd)"
OS="$(uname -s)"

case "$OS" in
    Linux* | Darwin*)
        # `-lveclite` resolves libveclite.{so,dylib}; rpath finds it at runtime.
        cc "$SRC" -I"$BUNDLE" -L"$BUNDLE" -lveclite -Wl,-rpath,"$BUNDLE" -o smoke_bin
        ./smoke_bin
        ;;
    MINGW* | MSYS* | CYGWIN*)
        # Link the *import library* (veclite.dll.lib), which records the shipped
        # `veclite.dll` name — linking the DLL file directly would record its PE
        # internal name instead. Then put the DLL on PATH to run.
        if command -v cl >/dev/null 2>&1; then
            # Dash-form flags avoid MSYS path-mangling; cl accepts them.
            cl -nologo -I"$BUNDLE" "$SRC" -Fe:smoke.exe "$BUNDLE/veclite.dll.lib"
        elif command -v zig >/dev/null 2>&1; then
            zig cc -target x86_64-windows-msvc "$SRC" -I"$BUNDLE" "$BUNDLE/veclite.dll.lib" -o smoke.exe
        else
            echo "smoke.sh: need MSVC 'cl' or 'zig' to link the import library" >&2
            exit 1
        fi
        PATH="$BUNDLE:$PATH" ./smoke.exe
        ;;
    *)
        echo "smoke.sh: unsupported OS '$OS'" >&2
        exit 1
        ;;
esac

echo "[ffi-smoke] link + run OK on $OS"
