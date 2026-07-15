# Bindings Specification

## ADDED Requirements

### Requirement: Python Binding with NumPy Zero-Copy
The Python package SHALL bind the Rust core via PyO3 with abi3 wheels installable without a Rust toolchain, borrow C-contiguous float32 NumPy buffers without copying on search and batch upsert, and release the GIL around every core call (SPEC-009 PY-001, PY-020..030).

#### Scenario: Batch upsert without copies
Given an (n, dim) float32 C-contiguous NumPy array
When upsert_batch is called with it
Then no per-row copy of the vector data occurs and all n vectors are stored

#### Scenario: Concurrent searches scale
Given a collection and 8 Python threads issuing searches
When throughput is compared against a single thread
Then aggregate throughput exceeds 4 times the single-thread rate

### Requirement: Python Exception Fidelity
Every VecLiteError variant MUST surface as a dedicated exception subclass of veclite.VecLiteError carrying the identical message text as the Rust display string (SPEC-009 PY-040).

#### Scenario: Dimension mismatch exception
Given a collection of dimension 384
When a 100-dim vector is upserted from Python
Then veclite.DimensionMismatch is raised with the same message as the Rust error
