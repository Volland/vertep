---
tgs: "0.1"
schemas:
  Component:
    properties:
      crate: text
      layer: {values: [core, authoring, semantics, storage, runtime, agent, distribution, verification, tooling, binding, facade]}
      status: {values: [planned, in-progress, stable], default: planned}
    edges:
      provides: [Capability, Requirement]
      uses: Component
      realizes: Decision
      scheduled_in: Milestone
    visualization: {color: "#16a34a", shape: rectangle}
  Milestone:
    properties:
      order: text
      status: {values: [planned, active, done], default: planned}
    edges:
      precedes: Milestone
    visualization: {color: "#db2777", shape: star}
  View:
    properties:
      status: {values: [draft, stable], default: draft}
    edges:
      describes: [Component, Capability, Decision, Requirement, Concept, Milestone, Constraint]
    visualization: {color: "#0891b2", shape: round-tag}
edgeTypes:
  provides:
    from: Component
    to: [Capability, Requirement]
    properties:
      confidence: {values: [full, partial]}
      note: text
  uses:
    from: Component
    to: Component
    properties:
      kind: {values: [required, optional, dev]}
      note: text
  realizes:
    from: Component
    to: Decision
    properties:
      confidence: {values: [full, partial]}
      note: text
  scheduled_in:
    from: [Component, Capability, Change]
    to: Milestone
    properties:
      note: text
  precedes:
    from: Milestone
    to: Milestone
    properties:
      note: text
  describes:
    from: View
    to: [Component, Capability, Decision, Requirement, Concept, Milestone, Constraint]
    properties:
      note: text
  specified_by:
    from: Capability
    to: Requirement
    properties:
      note: text
---
# Vertep Ontology

Project types and edges for Vertep on top of the bundled code ontology: crates as Components, delivery Milestones, and architecture Views that describe them.

## Questions it answers

Each type earns its place through a query the project asks.

- Which crate provides a requirement: `MATCH (c:Component)-[:provides]->(r:Requirement) RETURN c.title, r.title`.
- What a milestone delivers: `MATCH (x)-[:scheduled_in]->(m:Milestone {title: "M0 SCXML core"}) RETURN labels(x), x.title`.
- Which decisions a crate realizes, and why: `MATCH (c:Component)-[:realizes]->(d:Decision)-[:motivated_by]->(w) RETURN c.title, d.title, w.title`.
- Which views explain a component: `MATCH (v:View)-[:describes]->(c:Component) RETURN v.title, c.title`.
- What a capability is specified by: `MATCH (k:Capability)-[:specified_by]->(r:Requirement) RETURN k.title, r.title`. In tg 0.22, wiki links from `tg.md/` sections to `openspec:` targets do not resolve, open or archived, so `specified_by` stays unused for now; link docs to requirements with `openspec:` frontmatter and link code with `@tg:` annotations, which `tg trace` reads.
