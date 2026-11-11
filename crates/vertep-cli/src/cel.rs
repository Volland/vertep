//! The CEL type environment projection of a chart's types.
//!
//! Records become closed CEL message types ([`CelType::Record`], named after their named
//! type), tagged unions discriminated messages ([`CelType::Union`] with `tag` and `value`),
//! lists `list(T)`, optional values [`CelType::Optional`], and the primitives map to the
//! documented CEL types of `vertep-dm-cel`: `bool` to `bool`, `int` to `int`, `decimal` to the
//! exact `decimal` extension type, `string` and `iri` to `string`, `datetime` to
//! `google.protobuf.Timestamp`, and `value` to `dyn`. A recursive named type is expanded once;
//! its recursive occurrences are `dyn`, because CEL types are finite trees.

use vertep_core::types::{ChartTypes, Declarations, Primitive, Type};
use vertep_dm_cel::{CelType, TypeEnv};

/// The CEL type of `ty`, resolving named types in `decls`.
// @tg: implements:: [[openspec:type-system#CEL Type Environment Projection]]
pub fn cel_type(decls: &Declarations, ty: &Type) -> CelType {
    project(decls, ty, None, &mut Vec::new())
}

fn project(
    decls: &Declarations,
    ty: &Type,
    name: Option<&str>,
    stack: &mut Vec<String>,
) -> CelType {
    match ty {
        Type::Value => CelType::Dyn,
        Type::Primitive(p) => match p {
            Primitive::Bool => CelType::Bool,
            Primitive::Int => CelType::Int,
            Primitive::Decimal => CelType::Decimal,
            Primitive::String | Primitive::Iri => CelType::String,
            Primitive::Datetime => CelType::Timestamp,
        },
        Type::Named(n) => {
            if stack.iter().any(|s| s == n) {
                return CelType::Dyn;
            }
            let Some(def) = decls.lookup(n) else {
                return CelType::Dyn;
            };
            stack.push(n.clone());
            let t = project(decls, def, Some(n), stack);
            stack.pop();
            t
        }
        Type::List(item) => CelType::list(project(decls, item, None, stack)),
        Type::Optional(inner) => CelType::optional(project(decls, inner, None, stack)),
        Type::Record(fields) => {
            let fs: Vec<(String, CelType)> = fields
                .iter()
                .map(|f| (f.name.clone(), project(decls, &f.ty, None, stack)))
                .collect();
            CelType::record(name, fs.iter().map(|(n, t)| (n.as_str(), t.clone())))
        }
        Type::Union(variants) => {
            let vs: Vec<(String, Option<CelType>)> = variants
                .iter()
                .map(|v| {
                    (
                        v.tag.clone(),
                        v.payload.as_ref().map(|p| project(decls, p, None, stack)),
                    )
                })
                .collect();
            CelType::union(name, vs.iter().map(|(n, t)| (n.as_str(), t.clone())))
        }
    }
}

/// The CEL type environment of a type-checked chart: each context field bound under `ctx`,
/// each declared event's payload under `ev.data` (`null_type` for an event without payload).
/// An untyped chart gets the untyped environment, in which everything is `dyn`.
// @tg: implements:: [[openspec:type-system#CEL Type Environment Projection]]
pub fn type_env(types: &ChartTypes) -> TypeEnv {
    if !types.typed {
        return TypeEnv::untyped();
    }
    let decls = &types.declarations;
    let mut env = TypeEnv::typed();
    for f in &types.context {
        env = env.with_context_field(&f.name, cel_type(decls, &f.ty));
    }
    for (name, payload) in types.events.iter() {
        let t = payload.map_or(CelType::Null, |p| cel_type(decls, p));
        env = env.with_event(name, t);
    }
    env
}

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
