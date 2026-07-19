# Binding Metric and Text-Search Specification

## ADDED Requirements

### Requirement: Requested Metric Survives an Embedding Provider
Every binding SHALL create the collection with the metric the caller requested, whether or not an embedding provider is also given. A binding MUST NOT substitute the default metric silently; if a metric is genuinely unsupported for a provider, the call MUST fail with `InvalidArgument` naming the conflict rather than persisting a different collection (SPEC-009/010/011, SPEC-008 for the C ABI the Go and C# bindings inherit).

#### Scenario: Euclidean survives an auto-embed collection
Given a caller creating a collection with `dimension=4`, `metric="euclidean"` and `embedding_provider="bm25"`
When the collection is created and the file is inspected with `veclite inspect`
Then the collection reports `metric euclidean`, not `metric cosine`

#### Scenario: Every binding agrees
Given the same collection created through the Python, Node, C ABI, Go and C# bindings
When each reports the stored metric
Then all five report the metric that was requested, and match what the Rust API produces for the equivalent `CollectionOptions`

### Requirement: Out-of-Vocabulary Text Search Returns No Results
`search_text`, and the text lane of `hybrid_query`, SHALL return an empty result set when the query embeds to the zero vector — the normal outcome for a query whose terms are absent from the vocabulary. These entry points MUST NOT surface an error that refers to vectors or metrics, which the caller never supplied (SPEC-004, SPEC-005).

#### Scenario: Unknown terms yield no hits
Given an auto-embed collection holding the text "the quick brown fox"
When the caller runs `search_text("zzz unknownterm qqq", limit=3)`
Then the call succeeds and returns zero hits

#### Scenario: An explicit zero vector is still rejected
Given a cosine collection
When the caller passes an all-zero vector to `search` directly
Then the call fails with `InvalidArgument`, because cosine similarity is undefined for a zero vector and the caller chose both the vector and the metric

### Requirement: Lexical Default Is Documented
The README MUST state that the default `bm25` provider is lexical and that natural-language questions are better served by the dense `onnx` tier, so the flagship example does not set an expectation the default cannot meet (SPEC-005 §2).

#### Scenario: A reader knows which provider fits the query style
Given a reader following the README quickstart to build documentation search
When they read the auto-embed section
Then they learn that `bm25` scores lexical overlap and that natural-language questions want `onnx`, before choosing a provider
