//! The `vt` command line for Vertep charts.
//!
//! ```text
//! vt typecheck <chart>                                  type-check, every error with file:line:column
//! vt schema <chart> --format jsonschema|shacl|cel|rust  print a projection of the chart's types
//! vt validate <chart>                                   well-formedness and SHACL conformance
//! vt hash <chart>                                       print the chart version IRI
//! vt convert <chart> --to scxml|yaml|turtle|trig|nquads [-o <file>]
//! vt certify <chart>                                    run the Lean checker, print the certificate
//! vt cluster reshard <dir> <out>                        double a stopped cluster's shard count
//! ```
//!
//! A chart may be SCXML (`.scxml`), YAML (`.yaml`) or RDF (`.ttl`, `.trig`, `.nq`, `.nt`);
//! the surface is detected from the extension, else from the content. Every command exits 0
//! on success, 1 when the chart fails it, and 2 on usage and I/O errors.
//!
//! The library exposes the commands ([`commands`]), surface handling ([`surface`]) and the CEL
//! type environment projection of chart types ([`cel`]).

pub mod cel;
pub mod commands;
pub mod surface;

pub use commands::{Outcome, SchemaFormat};
pub use surface::Surface;
