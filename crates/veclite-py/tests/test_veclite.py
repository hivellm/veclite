"""Python binding tests (SPEC-009): quickstart, NumPy paths, exception fidelity."""

import numpy as np
import pytest

import veclite


def test_quickstart_memory():
    db = veclite.Database.memory()
    docs = db.create_collection("docs", dimension=3, metric="euclidean", quantization_bits=0)
    docs.upsert("a", [1.0, 0.0, 0.0], {"lang": "en"})
    docs.upsert("b", [0.0, 1.0, 0.0])
    assert len(docs) == 2

    hits = docs.search([0.9, 0.1, 0.0], limit=1)
    assert hits[0]["id"] == "a"
    assert hits[0]["payload"] == {"lang": "en"}

    got = docs.get("a")
    assert got["id"] == "a"
    assert got["vector"] == [1.0, 0.0, 0.0]
    assert docs.get("missing") is None
    assert docs.delete("a") is True
    assert len(docs) == 1


def test_numpy_batch_and_search():
    db = veclite.Database.memory()
    c = db.create_collection("v", dimension=4, metric="euclidean", quantization_bits=0)
    ids = [f"k{i}" for i in range(100)]
    vecs = np.zeros((100, 4), dtype=np.float32)
    vecs[:, 0] = np.arange(100, dtype=np.float32)
    c.upsert_batch(ids, vecs)
    assert len(c) == 100

    # Search with a NumPy float32 query (zero-copy borrow path).
    q = np.array([50.0, 0.0, 0.0, 0.0], dtype=np.float32)
    hits = c.search(q, limit=1)
    assert hits[0]["id"] == "k50"


def test_batch_with_payloads_and_filter():
    db = veclite.Database.memory()
    c = db.create_collection("v", dimension=2, metric="euclidean", quantization_bits=0)
    vecs = np.zeros((4, 2), dtype=np.float32)
    payloads = [{"lang": "en"}, {"lang": "pt"}, {"lang": "en"}, {"lang": "de"}]
    c.upsert_batch(["a", "b", "c", "d"], vecs, payloads)
    hits = c.search([0.0, 0.0], limit=10, filter={"must": [{"key": "lang", "match": {"value": "en"}}]})
    assert sorted(h["id"] for h in hits) == ["a", "c"]


def test_exception_hierarchy_and_messages():
    db = veclite.Database.memory()
    c = db.create_collection("docs", dimension=3, quantization_bits=0)

    # Dimension mismatch → dedicated subclass of VecLiteError.
    with pytest.raises(veclite.DimensionMismatch) as exc:
        c.upsert("x", [1.0, 2.0])  # dim 2 into dim-3
    assert issubclass(veclite.DimensionMismatch, veclite.VecLiteError)
    assert "3" in str(exc.value) and "2" in str(exc.value)

    # Missing collection.
    with pytest.raises(veclite.CollectionNotFound):
        db.collection("ghost")

    # Unknown provider.
    with pytest.raises(veclite.UnsupportedProvider):
        db.create_collection("t", dimension=8, embedding_provider="bm52")

    # Base class catches any subclass.
    with pytest.raises(veclite.VecLiteError):
        db.collection("ghost")


def test_auto_embed_text():
    db = veclite.Database.memory()
    c = db.create_collection("docs", dimension=64, embedding_provider="bm25")
    c.upsert_text("cats", "cats are small furry animals that meow")
    c.upsert_text("cars", "cars are fast vehicles with engines")
    hits = c.search_text("furry animals that meow", limit=1)
    assert hits[0]["id"] == "cats"


def test_aliases():
    db = veclite.Database.memory()
    db.create_collection("docs_v1", dimension=2, quantization_bits=0)
    db.create_alias("docs", "docs_v1")
    assert db.aliases() == [("docs", "docs_v1")]
    # Resolves transparently.
    db.collection("docs").upsert("x", [1.0, 0.0])
    assert len(db.collection("docs")) == 1


def test_hybrid_search():
    db = veclite.Database.memory()
    c = db.create_collection("h", dimension=2, metric="euclidean", quantization_bits=0)
    # BYO sparse vectors alongside dense.
    c.upsert("a", [0.0, 0.0])
    c.upsert("b", [1.0, 0.0])
    # Sparse lane via hybrid: dense-only degenerates to plain search here.
    hits = c.hybrid_search([0.0, 0.0], {"indices": [], "values": []}, limit=1)
    assert hits[0]["id"] == "a"


def test_metadata():
    assert isinstance(veclite.__version__, str)
    assert veclite.format_version == 1
