#!/usr/bin/env python3
"""VecLite Python quickstart (SPEC-009). Doubles as the clean-machine install
proof (REL-020): it imports the installed `veclite` wheel — no Rust toolchain —
and exercises the core flow. Exit non-zero on any surprise.

Run: `pip install veclite && python examples/quickstart.py`
"""

import tempfile
import os

import veclite


def main() -> int:
    # A durable single-file database — no server, no config (FR-01/02).
    path = os.path.join(tempfile.mkdtemp(), "app.veclite")
    db = veclite.Database.open(path)

    # BYO-vector collection, cosine metric.
    docs = db.create_collection("docs", 3, metric="cosine", quantization_bits=0)
    docs.upsert("a", [1.0, 0.0, 0.0], {"lang": "en"})
    docs.upsert("b", [0.0, 1.0, 0.0], {"lang": "fr"})
    docs.upsert("c", [0.9, 0.1, 0.0], {"lang": "en"})

    # k-NN search with a payload filter (SPEC-006).
    hits = docs.search(
        [1.0, 0.0, 0.0],
        limit=2,
        filter={"must": [{"key": "lang", "match": {"value": "en"}}]},
    )
    ids = [h["id"] for h in hits]
    assert ids == ["a", "c"], ids

    # An auto-embed (BM25) collection: text in, ranked ids out (SPEC-005).
    notes = db.create_collection("notes", 128, embedding_provider="bm25")
    notes.upsert_text("n1", "the quick brown fox")
    notes.upsert_text("n2", "a lazy sleeping dog")
    assert len(notes.search_text("quick fox", limit=2)) >= 1

    print(f"veclite {veclite.__version__}: quickstart OK ({ids})")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
