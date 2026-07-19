# Python quickstart

Install the wheel — no Rust toolchain, no build step:

```bash
pip install hivellm-veclite          # import name: veclite
```

The program below opens a durable single-file database, does a filtered k-NN
search over bring-your-own vectors (NumPy arrays are accepted zero-copy), and a
text search over an offline BM25 auto-embed collection. It doubles as the
clean-machine install proof — it imports only the installed wheel.

```python
{{#include ../../../examples/quickstart.py}}
```

Run it:

```bash
python examples/quickstart.py
# veclite 0.1.1: quickstart OK (['a', 'c'])
```

Hits are plain dicts (`hit["id"]`, `hit["score"]`, `hit["payload"]`). See
[SPEC-009](../../specs/SPEC-009-binding-python.md) for the full surface,
including NumPy zero-copy, GIL release, and the optional `veclite.aio` async
facade.
