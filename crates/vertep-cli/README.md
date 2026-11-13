<p align="center">
  <a href="https://volland.github.io/vertep/"><img src="https://volland.github.io/vertep/assets/img/logo.svg" alt="Vertep" width="120"></a>
</p>

<h1 align="center">vertep-cli</h1>

<p align="center">The <code>vt</code> command: type-check, validate, hash, convert and certify Vertep charts</p>

<p align="center">
  <a href="https://crates.io/crates/vertep-cli"><img src="https://img.shields.io/crates/v/vertep-cli.svg" alt="crates.io"></a>
  <a href="https://docs.rs/vertep-cli"><img src="https://img.shields.io/docsrs/vertep-cli" alt="docs.rs"></a>
  <a href="https://github.com/Volland/vertep/blob/main/LICENSE-MIT"><img src="https://img.shields.io/crates/l/vertep-cli.svg" alt="license"></a>
</p>

`vt` works on Vertep charts: W3C SCXML statecharts, the same charts written in YAML, or their
RDF projection. It type-checks a chart, prints its types as JSON Schema, SHACL or a CEL type
environment, checks well-formedness and the SHACL chart shapes, prints the chart's
content-addressed version IRI, converts between surfaces (refusing any conversion that would
lose something), generates a typed TypeScript module for the browser package, asks the Lean
model to certify a chart, and doubles the shard count of a stopped cluster.

The crate is built on `vertep-core`, `vertep-scxml`, `vertep-yaml`, `vertep-rdf`,
`vertep-dm-cel` and `vertep-cluster`. It is not part of the `vertep` facade. Install it as a
tool; depend on it as a library only to call the commands from Rust (`vertep_cli::commands`).

## Install

```sh
cargo install vertep-cli
vt --version
```

```text
vt 0.1.2
```

To use the commands from Rust:

```toml
[dependencies]
vertep-cli = "0.1"
```

The crate has no cargo features.

## Example

The examples below use this chart, saved as `light.scxml`:

```xml
<scxml xmlns="http://www.w3.org/2005/07/scxml" version="1.0" name="light" initial="red">
  <state id="red"><transition event="next" target="green"/></state>
  <state id="green"><transition event="next" target="yellow"/></state>
  <state id="yellow"><transition event="next" target="red"/></state>
</scxml>
```

`vt` picks the reader from the file extension: `.scxml` and `.xml` are SCXML, `.yaml` and
`.yml` are YAML, `.ttl`, `.trig`, `.nq` and `.nt` are RDF 1.2. With an unknown extension it
looks at the content. Every command exits 0 on success, 1 when the chart fails the command,
and 2 on usage or I/O errors. Results go to standard output, errors to standard error.

### vt validate

Checks the core well-formedness rules and the SHACL chart shapes, and prints the version IRI.

```sh
vt validate light.scxml
```

```text
light.scxml: ok: urn:vertep:chart:sha256:d0c0f4f1345d704f3cab0ecd7363a908f0c2d6b05354d0d22a38ba12c6c278f4
```

A broken chart gets one line per problem, with its position. Here, a chart that declares the state id `a` twice:

```text
dup.scxml:3:3: state id `a` is already declared at 2:3
```

### vt hash

Prints the chart version: the SHA-256 of the RDFC-1.0 canonical form of the chart's RDF
projection. It is computed from the chart, not the text, so the same chart in SCXML and YAML
has the same version.

```sh
vt hash light.scxml
vt convert light.scxml --to yaml -o light.yaml
vt hash light.yaml
```

```text
urn:vertep:chart:sha256:d0c0f4f1345d704f3cab0ecd7363a908f0c2d6b05354d0d22a38ba12c6c278f4
urn:vertep:chart:sha256:d0c0f4f1345d704f3cab0ecd7363a908f0c2d6b05354d0d22a38ba12c6c278f4
```

### vt convert

Converts to `scxml`, `yaml`, `turtle`, `trig` or `nquads`, to standard output or to `-o FILE`.

```sh
vt convert light.scxml --to yaml
```

```yaml
vertep: 1
name: light
datamodel:
  value: "null"
  implicit: true
initial: red
states:
  red:
    on:
      next: green
  green:
    on:
      next: yellow
  yellow:
    on:
      next: red
```

A conversion that would lose something writes nothing and exits 1. Here the input has an
attribute SCXML does not define (`colour="red"` on `<scxml>`):

```text
lossy.scxml: refusing to convert scxml to yaml: it would lose:
  - lossy.scxml:1:1: attribute `colour` is not defined on <scxml>
```

### vt typecheck

Runs the type checker and reports every error. A chart that declares no types is reported as
untyped, which is not an error.

```sh
vt typecheck light.scxml
```

```text
light.scxml: ok: untyped
```

This typed CEL chart, `counter.scxml`, assigns a string to an `int` field:

```xml
<scxml xmlns="http://www.w3.org/2005/07/scxml" xmlns:vt="https://w3id.org/vertep/vt#"
       version="1.0" name="counter" datamodel="cel" initial="idle">
  <vt:types>type Add = { n: int };
event add: Add;
event reset;</vt:types>
  <datamodel><data id="count" expr="0" vt:type="int"/></datamodel>
  <state id="idle">
    <transition event="add"><assign location="count" expr="ctx.count + ev.data.n"/></transition>
    <transition event="reset"><assign location="count" expr="'zero'"/></transition>
  </state>
</scxml>
```

```text
counter.scxml: state `idle` › transition 2 › <assign> 1 › expr: cannot assign string to `ctx.count` of type int
counter.scxml: 1 type error
```

With `expr="0"` in the `reset` transition it passes:

```text
counter.scxml: ok: 1 named types, 2 typed events, 1 context fields
```

### vt schema

Type-checks the chart, then prints one projection of its types: `--format cel`, `jsonschema`
(JSON Schema 2020-12, one document for the context and one per event payload) or `shacl`
(SHACL Core shapes in Turtle). `--format rust` exits 2 and points to the `include_chart!`
macro, which generates Rust types at compile time.

```sh
vt schema counter.scxml --format cel   # after the fix
```

```text
ctx.count: int
ev.data[add]: Add
ev.data[reset]: null_type
```

### vt gen ts

Prints a TypeScript module for `@vertep/web`: an event union, a context interface, the state
ids, the chart as a typed `ChartSpec` and a `create<Name>Actor` function. `--name` sets the
prefix.

```sh
vt gen ts light.scxml
```

```text
// Generated by `vt gen ts` from chart urn:vertep:chart:sha256:d0c0f4f1345d704f3cab0ecd7363a908f0c2d6b05354d0d22a38ba12c6c278f4. Do not edit.
import type { ActorRef, ActorSystem, ChartSpec, JsonValue } from "@vertep/web";

export type LightEvent = { type: string; [field: string]: JsonValue };
export type LightContext = JsonValue;
export type LightStateId = "red" | "green" | "yellow";
...
export function createLightActor(system: ActorSystem, key: string): ActorRef<LightEvent, LightContext, LightStateId> {
  return system.virtual(lightChart, key);
}
```

### vt certify

Asks the Lean model, not the Rust code, whether the chart is well-formed. `vt` writes the
chart's canonical N-Quads to a temporary file and runs the Lean-compiled checker `vt-certify`
on it. The checker recomputes the chart version, decides the Lean `WellFormed` predicate, and
prints a certificate as N-Triples.

`vt-certify` is not installed by `cargo install`. Build it from `lean/vertep-lean` in a
checkout of the [repository](https://github.com/Volland/vertep) (this needs the Lean toolchain
through `elan`; the version is pinned in `lean/vertep-lean/lean-toolchain`), then put it on
`PATH` or point `VT_CERTIFY` at it:

```sh
cd lean/vertep-lean && lake build && export VT_CERTIFY=$PWD/.lake/build/bin/vt-certify
```

Without it, `vt certify` exits 1 and prints those instructions. With it:

```sh
vt certify light.scxml
```

```text
<urn:vertep:chart:sha256:d0c0f4f1...c278f4> <https://w3id.org/vertep/vt#certifiedBy> <urn:vertep:cert:sha256:6708fdf7...bb1603b> .
<urn:vertep:cert:sha256:6708fdf7...bb1603b> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <https://w3id.org/vertep/vt#Certificate> .
<urn:vertep:cert:sha256:6708fdf7...bb1603b> <https://w3id.org/vertep/vt#theorem> "Vertep.checkWF_iff" .
<urn:vertep:cert:sha256:6708fdf7...bb1603b> <https://w3id.org/vertep/vt#theorem> "Vertep.step_deterministic" .
...
<urn:vertep:cert:sha256:6708fdf7...bb1603b> <https://w3id.org/vertep/vt#leanToolchain> "leanprover/lean4:v4.35.0" .
<urn:vertep:cert:sha256:6708fdf7...bb1603b> <https://w3id.org/vertep/vt#checkerHash> "sha256:cf351f3f...d48d90c" .
```

The IRIs are shortened here. The certificate names the theorems it relies on, the Lean
toolchain, the LeanRDF revision and the hash of the checker binary, so its IRI depends on the
checker you built. It hangs off the chart version with `vt:certifiedBy` and never changes the
chart's version.

### vt cluster reshard

Doubles the shard count of a stopped cluster, offline. `DIR` holds `cluster.meta`, the
tiramemsu shard files and `leases` (a dump of the live leases, one `<node> <shard> <epoch>`
line each). It refuses while any lease is live; otherwise it writes twice as many shard files
and the new metadata to `OUT`, copies every actor's journal to its new shard, and checks each
copy. `DIR` is left as it was.

```sh
vt cluster reshard data data-8
```

```text
resharding refused: the cluster is running, nodes node-a, node-b still hold shard leases
```

After the nodes stop and the lease dump is empty:

```text
resharded 4 to 8 shards: 0 actors, 0 entries copied to data-8
```

### did:webvh

`vt webvh` creates and hosts `did:webvh` DIDs with keys in a file key manager directory
(`--keys`). Beside the keys, `dids/<did>/` keeps the update-key names and the working log.
Logs are published only once they verify.

```sh
vt webvh create --domain example.org --keys keys --out site --pre-rotation
vt webvh verify site/example.org/.well-known/did.jsonl
```

```text
valid: did:webvh:QmXa...:example.org
versions: 1
version: 1-QmPd...
deactivated: false
```

`update`, `rotate` and `deactivate` add entries. `witness` adds a witness proof, and `publish`
writes to a directory, a git working copy (`--git`) or a directory followed by a command
(`--command`). `serve` serves a site directory. `object create` makes a durable object's DID,
whose custodians witness its log at the custody quorum. `custody change` prepares a custodian-set
change: the next log entry and the signed request for the writer.

### From Rust

Every subcommand is a function in `vertep_cli::commands` that returns an `Outcome` with the
standard output, standard error and exit code:

```rust
use vertep_cli::commands;

fn main() -> std::io::Result<()> {
    let path = std::env::temp_dir().join(format!("vt-readme-{}.scxml", std::process::id()));
    std::fs::write(
        &path,
        r#"<scxml xmlns="http://www.w3.org/2005/07/scxml" version="1.0" name="light" initial="red">
  <state id="red"><transition event="next" target="green"/></state>
  <state id="green"><transition event="next" target="yellow"/></state>
  <state id="yellow"><transition event="next" target="red"/></state>
</scxml>
"#,
    )?;
    let out = commands::hash(&path);
    assert_eq!(out.code, 0);
    assert_eq!(
        out.stdout.trim(),
        "urn:vertep:chart:sha256:d0c0f4f1345d704f3cab0ecd7363a908f0c2d6b05354d0d22a38ba12c6c278f4"
    );
    std::fs::remove_file(&path)
}
```

## Commands and key types

| Command | Library function | What it does |
| --- | --- | --- |
| `vt typecheck <chart>` | `commands::typecheck` | type-check; every error with its location |
| `vt schema <chart> --format cel\|jsonschema\|shacl\|rust` | `commands::schema` | print a projection of the chart's types |
| `vt validate <chart>` | `commands::validate` | well-formedness and SHACL conformance |
| `vt hash <chart>` | `commands::hash` | print the chart version IRI |
| `vt convert <chart> --to scxml\|yaml\|turtle\|trig\|nquads [-o FILE]` | `commands::convert` | convert, refusing lossy conversions |
| `vt gen ts <chart> [--name NAME]` | `commands::gen_ts` | TypeScript module for `@vertep/web` |
| `vt certify <chart>` | `commands::certify` | run the Lean checker, print the certificate |
| `vt cluster reshard <dir> <out>` | `commands::cluster_reshard` | double a stopped cluster's shard count |
| `vt webvh create\|update\|rotate\|deactivate\|verify\|witness\|publish\|serve` | `webvh` | author, verify, witness, publish and serve `did:webvh` DIDs |
| `vt webvh object create`, `vt webvh custody change` | `webvh::object_create`, `webvh::custody_change_cmd` | durable-object DIDs and custodian-set changes |

Library types: `Outcome` (stdout, stderr, exit code), `SchemaFormat`, `Surface` (surface
detection, reading and writing), and `cel` (the CEL type environment projection of chart
types).

## Known gaps

- `vt certify` needs the separate `vt-certify` checker, built with Lean from the repository.
- `vt schema --format rust` prints nothing; use the `include_chart!` macro of `vertep-macros`.
- Resharding is offline and only doubles the shard count.

## Documentation

- Website: https://volland.github.io/vertep/ ([The vt command](https://volland.github.io/vertep/docs/cli.html), [Verification](https://volland.github.io/vertep/docs/verification.html))
- API reference: https://docs.rs/vertep-cli
- Examples: https://volland.github.io/vertep/examples/ ([traffic light](https://volland.github.io/vertep/examples/traffic-light.html), [typed order](https://volland.github.io/vertep/examples/typed-order.html))

## License

MIT OR Apache-2.0, at your option.
