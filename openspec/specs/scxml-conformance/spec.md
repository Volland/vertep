# scxml-conformance Specification

## Purpose
Proves that Vertep implements SCXML by running the W3C SCXML Implementation Report Plan tests on the Rust core and on the Lean model, and checks that the two agree on generated charts, so conformance is a continuously enforced gate rather than a claim.

## Requirements

### Requirement: IRP Suite Import
The conformance harness SHALL import the W3C SCXML IRP test suite at a pinned revision, SHALL transform each templated test into its `ecmascript` and `null` datamodel variants where the test applies to that datamodel, and SHALL classify every test as mandatory, optional or manual as the IRP classifies it.

#### Scenario: Pinned revision
- **WHEN** the harness imports the suite
- **THEN** it uses the pinned revision and fails if the content hash of the downloaded suite differs

#### Scenario: Datamodel variants
- **WHEN** a templated test applies to the ecmascript datamodel
- **THEN** the harness produces an ecmascript variant ready to run

### Requirement: Pass Fail Protocol
The conformance harness SHALL count a test as passed when its session enters the top-level final state `pass`, as failed when it enters `fail`, and as failed with a reason when it exceeds its step or virtual-time budget or raises an interpreter error.

#### Scenario: Pass state reached
- **WHEN** a test session enters the final state `pass`
- **THEN** the test is recorded as passed

#### Scenario: Budget exceeded
- **WHEN** a test session neither passes nor fails within its budget
- **THEN** the test is recorded as failed with the reason budget exceeded

### Requirement: Test Host
The conformance harness SHALL provide an in-memory host that runs multiple sessions, delivers send effects through the SCXML Event I/O Processor between sessions, starts and cancels invoked `scxml` sessions, forwards autoforwarded events, delivers `done.invoke` events, and advances a virtual clock for delayed sends, without real time or network.

#### Scenario: Invoked child session
- **WHEN** a test invokes an inline `scxml` child that sends an event to `#_parent` and finishes
- **THEN** the parent receives that event and then `done.invoke.<id>`

#### Scenario: Delayed send with virtual time
- **WHEN** a test sends an event with a delay of one second and waits for it
- **THEN** the host advances virtual time and delivers the event without waiting in real time

### Requirement: Rust Core Conformance
The conformance harness SHALL run every mandatory test for the `null` and `ecmascript` datamodels on the Rust core in continuous integration, and the build SHALL fail when a mandatory test fails without an allow-list entry.

#### Scenario: Mandatory test failure breaks the build
- **WHEN** a mandatory test fails and is not on the allow-list
- **THEN** the conformance job fails and names the test

#### Scenario: All mandatory tests pass
- **WHEN** every mandatory test passes or is allow-listed
- **THEN** the conformance job succeeds

### Requirement: Lean Model Conformance
The conformance harness SHALL run the same tests on the Lean oracle, feeding it the datamodel answer tape recorded by the Rust run, and SHALL fail when the Lean model's verdict or trace differs from the Rust core's.

#### Scenario: Same verdict on both
- **WHEN** a mandatory test is run on the Rust core and on the Lean oracle
- **THEN** both report the same verdict and identical traces

#### Scenario: Divergence reported
- **WHEN** the Lean oracle requests a datamodel operation that differs from the recorded tape
- **THEN** the job fails and reports the test, the macrostep and both requests

### Requirement: Expected Deviation Allow List
The conformance harness SHALL read an allow-list in which every expected failure names the test, the datamodel and a reason, SHALL fail when an allow-listed test unexpectedly passes so the entry is removed, and SHALL fail when an entry names a test that does not exist.

#### Scenario: Unexpected pass fails
- **WHEN** an allow-listed test passes
- **THEN** the job fails and asks for the entry to be removed

#### Scenario: Entry without a reason
- **WHEN** an allow-list entry has no reason
- **THEN** the harness rejects the allow-list

### Requirement: Optional Test Reporting
The conformance harness SHALL run optional tests that the core can support, report their results separately, and exclude them and manual tests from the build gate.

#### Scenario: Optional failure does not break the build
- **WHEN** an optional test fails
- **THEN** it appears in the report as an optional failure and the job still succeeds

#### Scenario: Manual tests listed
- **WHEN** the report is produced
- **THEN** manual tests are listed as not run

### Requirement: Conformance Report
The conformance harness SHALL write a machine-readable report with, per test, the datamodel, classification, verdict on Rust, verdict on Lean and failure reason, plus totals per classification, and a human-readable summary.

#### Scenario: Report contents
- **WHEN** a conformance run finishes
- **THEN** the report contains one record per test variant and totals of passed, failed and allow-listed tests per classification

#### Scenario: Summary printed
- **WHEN** the run finishes in continuous integration
- **THEN** a summary with the totals and the failing tests is printed

### Requirement: Randomized Differential Testing
The conformance harness SHALL generate well-formed charts and input sequences, run them on the Rust core and on the Lean oracle, compare their traces, and shrink any counterexample to a minimal chart and input sequence that it stores as a regression test.

#### Scenario: Agreement on generated charts
- **WHEN** the configured number of generated charts and inputs are run on both
- **THEN** every trace pair is identical

#### Scenario: Counterexample shrunk and kept
- **WHEN** a generated case produces differing traces
- **THEN** the harness reports a shrunk chart and input sequence and stores them in the regression corpus

### Requirement: Round Trip Corpus Checks
The conformance harness SHALL check, for every chart of the conformance corpus, that the SCXML round trip and the RDF round trip return an equal chart and that the chart's projection conforms to the chart shapes.

#### Scenario: Corpus round trips
- **WHEN** the round-trip checks run over the corpus
- **THEN** every chart survives the SCXML and RDF round trips unchanged

#### Scenario: Corpus conforms to shapes
- **WHEN** the projection of every corpus chart is validated
- **THEN** every report conforms
