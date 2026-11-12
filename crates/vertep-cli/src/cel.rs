//! The CEL type environment projection of a chart's types, for `vt schema --format cel`.
//!
//! The projection itself ([`cel_type`], [`type_env`]) lives in `vertep-dm-cel`, where every
//! host instantiates CEL charts typed through it; it is re-exported here unchanged.

use vertep_core::types::ChartTypes;
use vertep_dm_cel::CelType;

pub use vertep_dm_cel::{cel_type, type_env};

/// The environment as text: one line per binding, `ctx.<field>: <type>` and
/// `ev.data[<event>]: <type>`.
pub fn describe(types: &ChartTypes) -> String {
    let env = type_env(types);
    let mut out = String::new();
    if !env.is_typed() {
        out.push_str("ctx: map(string, dyn)\nev.data: dyn\n");
        return out;
    }
    for f in &types.context {
        let t = env.context_field(&f.name).unwrap_or(CelType::Dyn);
        out.push_str(&format!("ctx.{}: {t}\n", f.name));
    }
    for (name, _) in types.events.iter() {
        if let Some(t) = env.event_payload(name) {
            out.push_str(&format!("ev.data[{name}]: {t}\n"));
        }
    }
    out
}
