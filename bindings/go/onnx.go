//go:build veclite_onnx

package veclite

// OnnxBuild reports whether this binary was built with the `veclite_onnx` tag,
// which links the ONNX-enabled C ABI library (SPEC-005 §6, EMB-040). Build with:
//
//	go build -tags veclite_onnx
//
// and place the ONNX-enabled `libveclite_ffi` (from
// `cargo build -p veclite-ffi --release --features onnx`) in the matching
// `lib/<goos>_<goarch>/`. With this build, `fastembed:<model>` and
// `fastembed:path:<dir>` embedding providers are available; the base build
// (default) rejects them and serves only vector-level operations (EMB-023).
const OnnxBuild = true
