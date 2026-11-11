# Views

Architecture views of Vertep, typed `View`: each explains a slice of the system with Mermaid diagrams and `describes` edges to the components, capabilities, decisions and milestones it covers.

- [[overview]] — system context, layers, crate map and one event end to end
- [[statecharts]] — chart IR, authoring surfaces, the step function, datamodels and types
- [[verification]] — Lean model, theorems, differential oracle, conformance and certificates
- [[rdf]] — the vt ontology modules, IR to RDF, SHACL and content addressing
- [[runtime]] — actors, lifecycle classes, mailboxes, transports, request, defer and timers
- [[persistence]] — event journal, storage traits, tiramemsu mapping and migrations
- [[agents]] — deciders, LLM and tool actors, grants and plan charts
- [[distribution]] — cluster membership, shards, QUIC, worker pools and DID federation
- [[browser]] — WebAssembly core, browser shell, OPFS journal and TypeScript API
