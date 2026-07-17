# Embeddings Specification

## ADDED Requirements

### Requirement: Opt-In ONNX Dense Embeddings
Dense neural embeddings SHALL be available only behind the onnx feature via fastembed, ship as separate heavy distribution artifacts that base packages never depend on, and support fully offline operation through local model paths (SPEC-005 EMB-040/041, SPEC-016 REL-021).

#### Scenario: Air-gapped model loading
Given a machine with no network access and a local MiniLM model directory
When a collection is created with provider fastembed:path:/models/minilm
Then text upserts and searches work without any network attempt

### Requirement: Graceful Degradation Without ONNX
A .veclite file whose collections use an onnx provider MUST open on builds without the onnx feature, serving vector-level reads and searches, with only text operations failing as UnsupportedProvider (SPEC-005 EMB-023).

#### Scenario: Vector search without the model
Given a file created with a fastembed provider and populated with vectors
When it is opened on a default build and searched with a caller-supplied vector
Then results return normally and only search_text fails with UnsupportedProvider
