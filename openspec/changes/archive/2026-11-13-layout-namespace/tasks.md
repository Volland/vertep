## 1. Graph first

- [x] 1.1 Record the decision `Layout is formatting in a reserved namespace` in `tg.md/decisions.md`, motivated by `Content-addressed chart versions`
- [x] 1.2 Update the `vertep-scxml` and `vertep-yaml` component sections and the `SCXML format` and `YAML format` capability sections

## 2. Namespace and SCXML

- [x] 2.1 Add `VL_NS = "https://w3id.org/vertep/layout#"` beside `VT_NS` in `vertep-core::ir` and re-export it from `vertep-scxml`
- [x] 2.2 Add `LossKind::Layout` (formatting) and drop `vl` attributes and elements in the reader, annotated `@tg: implements:: [[openspec:scxml-format#Layout Namespace]]`
- [x] 2.3 Tests for `Layout dropped from the IR`, `Strict mode accepts layout` and `Other foreign namespaces unchanged` in `vertep-scxml`, and `Layout does not change the chart version` in `vertep-rdf`, with `@tg: verifies::`

## 3. YAML

- [x] 3.1 Accept and ignore the top-level `layout` key, annotated `@tg: implements:: [[openspec:yaml-format#Layout Block]]`
- [x] 3.2 Tests for `Layout ignored by the reader`, `Layout never written` and `Layout does not change the chart version` with `@tg: verifies::`

## 4. Bindings

- [x] 4.1 Rebuild the JS/WASM bindings so vertepnyk reads laid-out charts
