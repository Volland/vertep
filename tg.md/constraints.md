---
type: Constraint
source: "External specifications, protocols and libraries; each section names its origin"
---
# Constraints

Rules Vertep did not choose: what the SCXML standard, RDF, SPARQL, the delivery path, the default store and the proof tooling impose on the design.

## SCXML interpretation algorithm

The W3C SCXML Recommendation (2015) defines a normative step-by-step interpretation algorithm and a conformance suite of about 200 tests.

- constrains:: [[decisions#SCXML semantics are normative]] {strength: must, source: "W3C SCXML Recommendation, Appendix D"}
- constrains:: [[decisions#Full SCXML first]] {strength: must, source: "W3C SCXML IRP tests"}

Most conformance tests assume the `ecmascript` datamodel, so passing the suite needs an ECMAScript datamodel.

## SCXML discards unmatched events

In SCXML an external event that enables no transition is dropped silently, which is safe for UI machines and unsafe for durable actors.

- constrains:: [[decisions#Non-blocking request and deferred events]] {strength: must, source: "W3C SCXML 3.13 and Appendix D"}

## RDF is open-world

A missing triple in RDF does not mean a fact is false, while execution needs closed facts such as exactly one initial state.

- constrains:: [[decisions#IR is the hub]] {strength: must, source: "RDF 1.2 Semantics"}
- constrains:: [[decisions#Modular vt ontology on RDF 1.2]] {strength: must, source: "RDF 1.2 Semantics; SHACL"}

## Proc macros run at compile time

A Rust procedural macro runs inside the compiler and must emit Rust types; it cannot read a triple store or go through RDF.

- constrains:: [[decisions#IR is the hub]] {strength: must, source: "Rust proc_macro"}
- constrains:: [[decisions#One IR type language]] {strength: should, source: "Rust proc_macro"}

## SPARQL Update has no compare-and-set

SPARQL 1.1 Update offers no conditional append, and transaction support varies between stores, so a generic endpoint cannot promise linearizable appends.

- constrains:: [[decisions#Two storage traits with tiramemsu as default]] {strength: must, source: "SPARQL 1.1 Update"}

## Relays deliver at least once

Store-and-forward mailboxes such as DIDComm mediators and TSP intermediaries redeliver after crashes and timeouts, so exactly-once delivery is impossible.

- constrains:: [[decisions#Event-sourced actors]] {strength: must, source: "DIDComm v2 mediators; ToIP TSP"}
- constrains:: [[decisions#Identity, mailbox and transport layers]] {strength: must, source: "DIDComm v2 mediators; ToIP TSP"}

## Tiramemsu has a single writer per file

Tiramemsu serializes every transaction through one writer per database file and has no network server.

- constrains:: [[decisions#Two storage traits with tiramemsu as default]] {strength: must, source: "tiramemsu lat.md/architecture.md#Connections and Concurrency"}
- constrains:: [[decisions#Shard-based placement]] {strength: must, source: "tiramemsu lat.md/constraints.md#Single Writer"}

The single writer is what makes a compare-and-set journal append atomic, and also why one file must hold one shard rather than a whole node.

## Aeneas supports a Rust subset

Aeneas translates only a restricted subset of safe Rust to Lean, with limited traits and no async.

- constrains:: [[decisions#Lean verification bridge]] {strength: should, source: "Aeneas"}
- constrains:: [[decisions#No-std core and tokio shell]] {strength: should, source: "Aeneas"}
