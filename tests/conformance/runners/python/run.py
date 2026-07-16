#!/usr/bin/env python3
"""Python binding conformance runner (SPEC-015 §3, TST-020..023).

Loads the shared YAML corpus and the committed ``golden.json`` and drives them
through the installed ``veclite`` wheel, reproducing the Rust reference runner's
observations exactly (orderings/ids exact, scores within 1e-5). Exit non-zero on
any divergence.

Usage: ``python run.py [corpus_dir]`` (defaults to ``tests/conformance/corpus``).
"""

from __future__ import annotations

import gc
import json
import os
import sys
import tempfile
from pathlib import Path

import yaml

import veclite

TOL = 1e-5

# Native exception class name → stable error code (identical to the FFI/Rust
# codes, so a corpus `error:` assertion is language-independent).
CLASS_TO_CODE = {
    "CollectionNotFound": "COLLECTION_NOT_FOUND",
    "VectorNotFound": "VECTOR_NOT_FOUND",
    "AlreadyExists": "ALREADY_EXISTS",
    "DimensionMismatch": "DIMENSION_MISMATCH",
    "Locked": "LOCKED",
    "WalPending": "WAL_PENDING",
    "ReadOnly": "READ_ONLY",
    "Closed": "CLOSED",
    "Corrupt": "CORRUPT",
    "UnsupportedFormat": "UNSUPPORTED_FORMAT_VERSION",
    "UnsupportedProvider": "UNSUPPORTED_PROVIDER",
    "InvalidArgument": "INVALID_ARGUMENT",
    "IoError": "IO",
}


class OpError(Exception):
    """An operation failed with a stable VecLite error code."""

    def __init__(self, code: str):
        super().__init__(code)
        self.code = code


def code_of(exc: BaseException) -> str:
    return CLASS_TO_CODE.get(type(exc).__name__, "ERROR")


# ── numeric-tolerant comparison (mirrors the Rust runner) ────────────────────

def eq_tol(a, b) -> bool:
    """Deep equality; numbers within TOL, lists exact-length, dicts exact-keys."""
    if isinstance(a, bool) or isinstance(b, bool):
        return a == b
    if isinstance(a, (int, float)) and isinstance(b, (int, float)):
        return abs(float(a) - float(b)) <= TOL
    if isinstance(a, list) and isinstance(b, list):
        return len(a) == len(b) and all(eq_tol(x, y) for x, y in zip(a, b))
    if isinstance(a, dict) and isinstance(b, dict):
        return a.keys() == b.keys() and all(eq_tol(v, b[k]) for k, v in a.items())
    return a == b


def matches_subset(want, got) -> bool:
    """Inline-expect match: dict `want` asserts only its own keys; else eq_tol."""
    if isinstance(want, dict) and isinstance(got, dict):
        return all(k in got and matches_subset(v, got[k]) for k, v in want.items())
    if isinstance(want, list) and isinstance(got, list):
        return len(want) == len(got) and all(matches_subset(x, y) for x, y in zip(want, got))
    if isinstance(want, bool) or isinstance(got, bool):
        return want == got
    if isinstance(want, (int, float)) and isinstance(got, (int, float)):
        return abs(float(want) - float(got)) <= TOL
    return want == got


# ── operation dispatch (produces the shared observation shapes) ──────────────

def hits_obs(hits) -> dict:
    return {"ids": [h["id"] for h in hits], "scores": [h["score"] for h in hits]}


def create_collection(db, a) -> dict:
    provider = a.get("auto_embed")
    metric = a.get("metric", "cosine")
    bits = a.get("quantization_bits")
    db.create_collection(
        a["name"],
        a["dimension"],
        metric=metric,
        quantization_bits=bits,
        embedding_provider=provider,
    )
    return {}


def get_obs(coll, a) -> dict:
    p = coll.get(a["id"])
    if p is None:
        return {"result": None}
    return {"result": {"id": p["id"], "vector": list(p["vector"]), "payload": p["payload"]}}


def stats_obs(coll) -> dict:
    s = coll.stats()
    return {"value": {
        "dimension": s["dimension"],
        "len": s["len"],
        "tombstones": s["tombstones"],
        "auto_embed": s["auto_embed"],
    }}


def search_obs(coll, a) -> dict:
    hits = coll.search(
        a["vector"],
        limit=a.get("limit", 10),
        ef_search=a.get("ef_search"),
        with_payload=a.get("with_payload", True),
        with_vector=a.get("with_vector", False),
        filter=a.get("filter"),
    )
    return hits_obs(hits)


def hybrid_obs(coll, a) -> dict:
    hits = coll.hybrid_search(
        a["dense"],
        a["sparse"],
        limit=a.get("limit", 10),
        alpha=a.get("alpha", 0.5),
        rrf_k=a.get("rrf_k", 60.0),
    )
    return hits_obs(hits)


def scroll_obs(coll, a) -> dict:
    page = coll.scroll(
        limit=a.get("limit", 100),
        offset_id=a.get("offset_id"),
        filter=a.get("filter"),
    )
    return {"ids": [p["id"] for p in page["points"]], "next_cursor": page["next_cursor"]}


def chunk_obs(a) -> dict:
    d = veclite.chunk(a["text"], a.get("max_chars", 2048), a.get("overlap", 128))
    return {"result": [{"text": c["text"], "start": c["start"], "end": c["end"]} for c in d]}


def execute(db, op: str, a: dict) -> dict:
    """Run one op, returning its canonical observation. Raises OpError on a
    VecLite failure (mapped to a stable code)."""
    try:
        if op == "create_collection":
            return create_collection(db, a)
        if op == "delete_collection":
            db.delete_collection(a["name"])
            return {}
        if op == "list_collections":
            return {"ids": db.list_collections()}
        if op == "create_alias":
            db.create_alias(a["alias"], a["target"])
            return {}
        if op == "delete_alias":
            db.delete_alias(a["alias"])
            return {}
        if op == "upsert":
            db.collection(a["collection"]).upsert(a["id"], a["vector"], a.get("payload"), a.get("sparse"))
            return {}
        if op == "upsert_batch":
            coll = db.collection(a["collection"])
            pts = a["points"]
            ids = [p["id"] for p in pts]
            vecs = [p["vector"] for p in pts]
            payloads = [p.get("payload") for p in pts]
            coll.upsert_batch(ids, vecs, payloads if any(x is not None for x in payloads) else None)
            return {}
        if op == "upsert_text":
            db.collection(a["collection"]).upsert_text(a["id"], a["text"], a.get("payload"))
            return {}
        if op == "refit":
            db.collection(a["collection"]).refit()
            return {}
        if op == "get":
            return get_obs(db.collection(a["collection"]), a)
        if op == "delete":
            return {"value": db.collection(a["collection"]).delete(a["id"])}
        if op == "len":
            return {"value": len(db.collection(a["collection"]))}
        if op == "stats":
            return stats_obs(db.collection(a["collection"]))
        if op == "search":
            return search_obs(db.collection(a["collection"]), a)
        if op == "search_text":
            return hits_obs(db.collection(a["collection"]).search_text(a["query"], a.get("limit", 10)))
        if op == "hybrid_search":
            return hybrid_obs(db.collection(a["collection"]), a)
        if op == "scroll":
            return scroll_obs(db.collection(a["collection"]), a)
        if op == "chunk":
            return chunk_obs(a)
        raise RuntimeError(f"unknown op {op!r}")
    except veclite.VecLiteError as exc:
        raise OpError(code_of(exc)) from exc


# ── case execution ───────────────────────────────────────────────────────────

def open_db(path):
    return veclite.Database.memory() if path is None else veclite.Database.open(str(path))


def run_case(case: dict, golden: dict) -> list[str]:
    mode = case.get("mode", "both")
    modes = {"both": [None, True], "memory": [None], "file": [True]}[mode]
    expected = golden.get(case["id"])
    errors: list[str] = []
    for file_mode in modes:
        errors += run_case_in_mode(case, file_mode, expected)
    return errors


def run_case_in_mode(case: dict, file_mode, golden) -> list[str]:
    errors: list[str] = []
    path = None
    if file_mode:
        fd, name = tempfile.mkstemp(suffix=".veclite")
        os.close(fd)
        os.unlink(name)
        path = Path(name)
    db = open_db(path)
    try:
        idx = 0
        for i, step in enumerate(case["steps"]):
            op = step["op"]
            where = f"step {i} `{op}`"
            if op == "reopen":
                if path is None:
                    errors.append(f"{case['id']}: {where}: reopen requires file mode")
                    break
                db.checkpoint()
                db = None
                gc.collect()
                db = open_db(path)
                continue

            try:
                obs = execute(db, op, step.get("args", {}) or {})
            except OpError as e:
                obs = {"error": e.code}

            expect = step.get("expect")
            if expect is not None:
                for key, want in expect.items():
                    if key == "error":
                        if obs.get("error") != want:
                            errors.append(f"{case['id']}: {where}: expected error {want}, got {obs}")
                    elif not matches_subset(want, obs.get(key)):
                        errors.append(f"{case['id']}: {where}: `{key}`: expected {want}, got {obs.get(key)}")
            elif "error" in obs:
                errors.append(f"{case['id']}: {where}: unexpected error {obs['error']}")

            if golden is not None:
                if idx >= len(golden):
                    errors.append(f"{case['id']}: {where}: golden has no entry (re-bless)")
                elif not eq_tol(golden[idx], obs):
                    errors.append(f"{case['id']}: {where}: golden mismatch: {golden[idx]} != {obs}")
            idx += 1
    finally:
        db = None
        gc.collect()
        if path is not None:
            for p in (path, path.with_name(path.name + "-wal")):
                try:
                    p.unlink()
                except OSError:
                    pass
    return errors


def main() -> int:
    here = Path(__file__).resolve()
    default_corpus = here.parents[2] / "corpus"
    corpus = Path(sys.argv[1]) if len(sys.argv) > 1 else default_corpus

    golden = json.loads((corpus / "golden.json").read_text(encoding="utf-8"))
    files = sorted(corpus.glob("*.yaml"))
    if not files:
        print(f"[conformance:py] no *.yaml under {corpus}", file=sys.stderr)
        return 1

    total = failed = 0
    for f in files:
        suite = yaml.safe_load(f.read_text(encoding="utf-8"))
        for case in suite["cases"]:
            total += 1
            errs = run_case(case, golden)
            if errs:
                failed += 1
                for e in errs:
                    print(f"[conformance:py] FAIL {e}", file=sys.stderr)

    if failed:
        print(f"[conformance:py] {failed}/{total} cases FAILED", file=sys.stderr)
        return 1
    print(f"[conformance:py] PASS — {total} cases across {len(files)} files", file=sys.stderr)
    return 0


if __name__ == "__main__":
    sys.exit(main())
