# Testing Specification

## ADDED Requirements

### Requirement: Cross-Binding Conformance Corpus
One YAML corpus SHALL define operations and expected outcomes executed by runners in every supported language, with exact id-set and ordering comparisons and 1e-5 score tolerance; a binding is release-blocked until the corpus passes on its full platform matrix (SPEC-015 TST-020..023, PRD FR-65).

#### Scenario: Binding divergence caught
Given the corpus passing on the Rust reference runner
When a binding returns a different error code for the same invalid operation
Then that binding's conformance job fails naming the corpus case id

### Requirement: Toolchain-Free Installation
pip install veclite and npm install veclite MUST succeed and run the quickstart on clean machines without a Rust toolchain across the supported OS and architecture matrix (PRD FR-66, SPEC-016 REL-020).

#### Scenario: Clean container quickstart
Given a fresh container image with only Python or Node installed
When the package is installed from built artifacts and the quickstart runs
Then installation performs no compilation and the quickstart prints search hits
