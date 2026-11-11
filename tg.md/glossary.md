---
type: Concept
status: draft
---
# Glossary

Terms with one exact meaning in Vertep.

## Virtual actor

An actor with a durable IRI identity that always exists logically, is activated on its first message and deactivated when idle, and keeps its state in a journal.

- defines:: [[decisions#IRI identity and lifecycle classes]]

It is reached only by `<send>` or `vt:request`, never by `<invoke>`, because no state owns its lifetime.

## Invoked actor

A child actor started by `<invoke>`, addressed by its parent IRI plus invoke id, and cancelled when the invoking state exits.

- defines:: [[decisions#IRI identity and lifecycle classes]]

## Stateless worker

A pooled actor with many activations per address and no journal, used for LLM calls, tools and heavy computation; its results are journaled by the caller.

- defines:: [[decisions#IRI identity and lifecycle classes]]

## Mailbox

A durable inbox for one or more actors that outlives activations; it can be a local queue, a store-backed queue or a DIDComm or TSP mediator.

- defines:: [[decisions#Identity, mailbox and transport layers]]

## Decider

The pluggable chooser behind a `vt:decide` state: guards, an LLM, a human or another actor, which may only pick one of the currently enabled events.

- defines:: [[decisions#Deciders at choice points]]

## Shard

The unit of placement in a cluster: the set of actors whose IRI hashes to it, leased to one node and stored in one journal file.

- defines:: [[decisions#Shard-based placement]]
