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

A durable inbox for one or more actors that outlives activations; it can be a local queue, a store-backed queue, a DIDComm mediator or a TSP mediator such as `vt-mediator`.

- defines:: [[decisions#Identity, mailbox and transport layers]]

## Decider

The pluggable chooser behind a `vt:decide` state: guards, an LLM, a human or another actor, which may only pick one of the currently enabled events.

- defines:: [[decisions#Deciders at choice points]]

## Shard

The unit of placement in a cluster: the set of actors whose IRI hashes to it, leased to one node and stored in one journal file.

- defines:: [[decisions#Shard-based placement]]

## Durable object

A virtual actor with a `did:webvh` address and a hash-chained, signed journal kept by its custodians, so it survives any one host or organisation and has one writer at a time.

- defines:: [[decisions#Durable objects are DID virtual actors with verifiable journals]]

## Custodian

A party named in a durable object's DID document that keeps its journal: a host may activate and write the object, a keeper only stores encrypted entries.

- defines:: [[decisions#Keepers store ciphertext under a per-object content key]]

## Custody epoch

The numbered term in which one host is a durable object's only writer, won by promises from a majority of its custodians and recorded in the journal.

- defines:: [[decisions#Custody epochs are won by quorum promises]]

## Keeper

A storage-only custodian: it stores, orders and verifies a durable object's entries by their clear headers but cannot read their encrypted bodies.

- defines:: [[decisions#Keepers store ciphertext under a per-object content key]]

## Vertep

The travelling puppet theatre of Ukrainian Christmas, whose puppets move only along slots cut in the stage floor; the project's name and its picture of a chart: a decider picks a slot, it never cuts one.

- defines:: [[decisions#Deciders at choice points]] {note: "the puppeteer chooses among the slots the stage offers"}
- defines:: [[decisions#Documentation site is plain static HTML]] {note: "the site's marks are the vertep star and goat"}

## Agent protocol

A hash-identified choreography of roles, messages and states that agents publish and follow; an agent's interface is its role's projection of it.

- defines:: [[decisions#A protocol is a role-based choreography and the interface is its projection]]
- defines:: [[decisions#Protocols are identified by the hash of their graph]]

## Protocol space

The tree of protocol instances under one root genesis, sharing one hash-linked history of messages that are each sealed per recipient.

- defines:: [[decisions#A protocol space is logical, sealed pairwise and hash-linked]]
- defines:: [[decisions#Sub-protocols make a space a tree of instances]]

## Role projection

A protocol restricted to what one role sends and receives, with every other step silent; it is what a role's checker runs.

- defines:: [[decisions#Choreographies are one global chart projected in a restricted subset]]

## Kolyada

The name of Vertep's agent protocol layer, after the Ukrainian carol that singers perform door to door from a known script: agents that follow a published script, each in its role.

- defines:: [[decisions#A protocol is a role-based choreography and the interface is its projection]]
- defines:: [[decisions#The protocol core is a crate with one-way dependencies]] {note: "the crate is vertep-kolyada"}
