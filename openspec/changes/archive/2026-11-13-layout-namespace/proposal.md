## Why

vertepnyk, the visual statechart designer, keeps each chart's visual layout (state boxes, transition routes, label positions) inline in the chart file, so one file is both the chart and its diagram. Today Vertep would read that layout as content: in SCXML a `vl:x` attribute becomes a foreign extension node, is hashed into the chart version and fails strict mode; in YAML an unknown top-level `layout` key is a schema error. Moving a box must never change what a chart means, its version IRI, or whether it loads strictly.

## What Changes

- Reserve the layout namespace `https://w3id.org/vertep/layout#` (conventional prefix `vl`), exported as `VL_NS` beside `VT_NS` in `vertep-core::ir`.
- **SCXML.** Every attribute and element in the `vl` namespace is formatting: the reader drops it, lists it in `LossReport::formatting` as the new `LossKind::Layout`, and never fails strict mode for it. The `xmlns:vl` declaration is not an extension either. vertepnyk writes `vl:x`, `vl:y`, `vl:w`, `vl:h` and `vl:collapsed` on `<state>`, `<parallel>`, `<final>` and `<history>`, and `vl:route="x1,y1 x2,y2 ..."` and `vl:label="x,y"` on `<transition>`; any other `vl` attribute or element is dropped the same way. Other foreign namespaces are unchanged: they stay `ForeignExtension` content.
- **YAML.** An optional top-level `layout` key with any value is accepted and ignored by the reader. vertepnyk writes `layout: { states: { <id>: { x, y, w, h, collapsed } }, transitions: { "<source>#<index>": { route: [[x, y], ...], label: [x, y] } } }`. The writer never emits it.
- Layout therefore never reaches the IR, so it never changes the chart hash or the version IRI.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `scxml-format`: adds the layout namespace requirement.
- `yaml-format`: the chart document allows an optional `layout` block; adds the layout block requirement.

## Impact

- `vertep-core` (`ir::VL_NS`), `vertep-scxml` (`LossKind::Layout`, reader), `vertep-yaml` (reader top-level keys), tests in `vertep-scxml`, `vertep-yaml` and `vertep-rdf` (hash), and a rebuild of the JS/WASM bindings for vertepnyk.
- `LossKind` is `#[non_exhaustive]`, so the new variant is not a breaking change. No writer, IR shape, RDF projection or hash changes.
