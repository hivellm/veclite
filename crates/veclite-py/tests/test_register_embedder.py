"""Custom Python embedders (SPEC-009 PY-013): registration, auto-embed routing,
state persistence, exception chaining, and validation."""

import gc

import numpy as np
import pytest

import veclite


class HashEmbedder:
    """A tiny deterministic embedder: hashed token counts, L2-normalized."""

    def __init__(self, dim):
        self._dim = dim
        self._docs = 0

    @property
    def dimension(self):
        return self._dim

    def embed(self, text):
        v = np.zeros(self._dim, dtype=np.float32)
        for tok in text.split():
            v[hash(tok) % self._dim] += 1.0
        n = np.linalg.norm(v)
        return v / n if n else v

    def fit(self, corpus):
        self._docs = len(corpus)

    def export_state(self):
        return self._docs.to_bytes(4, "little")

    def import_state(self, state):
        self._docs = int.from_bytes(bytes(state), "little")


def test_custom_embedder_powers_text_search():
    db = veclite.Database.memory()
    db.register_embedder("myhash", HashEmbedder(16))
    docs = db.create_collection("d", dimension=16, embedding_provider="myhash")
    docs.upsert_text("a", "the quick brown fox")
    docs.upsert_text("b", "a lazy sleeping dog")
    hits = docs.search_text("quick fox", limit=2)
    assert {h["id"] for h in hits} <= {"a", "b"}
    assert any(h["id"] == "a" for h in hits)


def test_custom_embedder_returning_a_list_also_works():
    class ListEmbedder:
        @property
        def dimension(self):
            return 4

        def embed(self, text):
            return [float(len(text)), 0.0, 0.0, 0.0]

    db = veclite.Database.memory()
    db.register_embedder("listy", ListEmbedder())
    c = db.create_collection("c", dimension=4, embedding_provider="listy")
    c.upsert_text("x", "hello")
    assert len(c) == 1


def test_embedder_exception_chains_the_original_traceback():
    class BadEmbedder:
        @property
        def dimension(self):
            return 8

        def embed(self, text):
            raise ValueError("boom in python")

    db = veclite.Database.memory()
    db.register_embedder("bad", BadEmbedder())
    c = db.create_collection("bad", dimension=8, embedding_provider="bad")
    with pytest.raises(veclite.VecLiteError) as excinfo:
        c.upsert_text("x", "hello")
    cause = excinfo.value.__cause__
    assert isinstance(cause, ValueError)
    assert "boom in python" in str(cause)


def test_register_embedder_validates_the_object():
    db = veclite.Database.memory()

    class NoEmbed:
        @property
        def dimension(self):
            return 4

    with pytest.raises(veclite.InvalidArgument):
        db.register_embedder("no_embed", NoEmbed())

    class NoDim:
        def embed(self, text):
            return [0.0]

    with pytest.raises(veclite.InvalidArgument):
        db.register_embedder("no_dim", NoDim())

    class ZeroDim:
        @property
        def dimension(self):
            return 0

        def embed(self, text):
            return []

    with pytest.raises(veclite.InvalidArgument):
        db.register_embedder("zero_dim", ZeroDim())


def test_embedder_state_survives_persistence(tmp_path):
    path = str(tmp_path / "emb.veclite")
    db = veclite.Database.open(path)
    db.register_embedder("myhash", HashEmbedder(16))
    docs = db.create_collection("d", dimension=16, embedding_provider="myhash")
    docs.upsert_text("a", "alpha beta gamma")
    docs.upsert_text("b", "delta epsilon")
    docs.refit()  # exercises fit() + re-embed through the Python callback
    db.checkpoint()
    before = {h["id"] for h in docs.search_text("alpha", limit=2)}

    # Drop the first handle so its single-writer lock is released before reopen.
    del docs, db
    gc.collect()

    # Reopen: the collection's provider is "myhash", which must be re-registered
    # before use (a Python embedder can't be persisted, only its state).
    db2 = veclite.Database.open(path)
    db2.register_embedder("myhash", HashEmbedder(16))
    docs2 = db2.collection("d")
    after = {h["id"] for h in docs2.search_text("alpha", limit=2)}
    assert before == after
