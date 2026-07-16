"""VecLite — embedded, single-file, in-process vector database (SPEC-009).

The compiled core lives in ``veclite._veclite``; this package re-exports it so
``import veclite`` gives you ``Database``, ``Collection``, ``chunk``, and the
exception hierarchy. The optional async facade ``veclite.aio`` (PY-031) is
imported lazily so plain synchronous use pays nothing for it.
"""

from ._veclite import *  # noqa: F401,F403 — public classes, chunk, exceptions
from ._veclite import __version__, format_version  # noqa: F401 — dunder/underscore-safe

__all__ = [
    "Database",
    "Collection",
    "chunk",
    "aio",
    "format_version",
    "__version__",
    # exception hierarchy
    "VecLiteError",
    "CollectionNotFound",
    "VectorNotFound",
    "AlreadyExists",
    "DimensionMismatch",
    "Locked",
    "WalPending",
    "ReadOnly",
    "Closed",
    "Corrupt",
    "UnsupportedFormat",
    "UnsupportedProvider",
    "InvalidArgument",
    "IoError",
]


def __getattr__(name):
    # PEP 562 lazy attribute: `veclite.aio` imports only on first access.
    if name == "aio":
        import importlib

        module = importlib.import_module("veclite.aio")
        globals()["aio"] = module
        return module
    raise AttributeError(f"module 'veclite' has no attribute {name!r}")
