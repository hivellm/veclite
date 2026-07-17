//go:build !veclite_onnx

package veclite

// OnnxBuild reports whether this binary links the ONNX-enabled C ABI library.
// The default (base) build is `false`: it stays lean and network-free (NFR-08).
// See onnx.go for the opt-in `veclite_onnx` build.
const OnnxBuild = false
