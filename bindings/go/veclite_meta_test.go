package veclite

import "testing"

func TestMeta(t *testing.T) {
	if Version() == "" {
		t.Fatal("empty version")
	}
	if AbiVersion() == 0 {
		t.Fatal("abi version must be >= 1")
	}
	t.Logf("veclite %s abi=%d format=%d", Version(), AbiVersion(), FormatVersion())
}
