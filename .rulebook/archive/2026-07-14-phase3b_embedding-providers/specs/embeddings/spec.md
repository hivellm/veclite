# Embeddings Specification

## ADDED Requirements

### Requirement: Pure-Rust Sparse Providers
The default build SHALL ship bm25 (default provider, k1=1.5, b=0.75), tfidf, bow, and char_ngram providers with scores matching the Vectorizer server within 1e-5 given identical vocabulary state, and unknown provider names MUST fail with UnsupportedProvider listing the available ones — never a silent fallback (SPEC-005 EMB-002, EMB-021).

#### Scenario: Unknown provider rejected
Given a create_collection call with auto_embed provider name bm52
When the collection is created
Then the call fails with UnsupportedProvider listing bm25, tfidf, bow, char_ngram

### Requirement: Self-Contained Vocabulary Persistence
Auto-embed collections MUST persist provider vocabulary state inside the file so a reopened .veclite returns identical search_text results on any machine with no network access (SPEC-005 EMB-020, EMB-030).

#### Scenario: Reopen preserves scoring
Given an auto-embed bm25 collection with 1000 indexed texts
When the database is closed, copied to another location, and reopened
Then search_text returns identical hits and scores for the same queries
