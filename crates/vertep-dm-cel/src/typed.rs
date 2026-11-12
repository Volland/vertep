//! Typed instantiation: the CEL type environment projection of a chart's types, and the load
//! every host goes through when it creates a CEL datamodel for a chart
//! ([[decisions#Hosts instantiate CEL charts typed]]).
//!
//! Records become closed CEL message types ([`CelType::Record`], named after their named
//! type), tagged unions discriminated messages ([`CelType::Union`] with `tag` and `value`),
//! lists `list(T)`, optional values [`CelType::Optional`], and the primitives map to the
//! documented CEL types of this crate: `bool` to `bool`, `int` to `int`, `decimal` to the
//! exact `decimal` extension type, `string` and `iri` to `string`, `datetime` to
//! `google.protobuf.Timestamp`, and `value` to `dyn`. A recursive named type is expanded once;
//! its recursive occurrences are `dyn`, because CEL types are finite trees.

use alloc::string::String;
use alloc::vec::Vec;

use vertep_core::ir::Chart;
use vertep_core::types::{
    BasicTyper, ChartTypes, Declarations, Primitive, Type, check_chart, is_typed,
};

use crate::datamodel::{CEL, CelDatamodel};
use crate::error::Limits;
use crate::load::{LoadError, LoadErrors, LoadOptions, load};
use crate::policy::Policy;
use crate::types::{CelType, TypeEnv};

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

/// The CEL datamodel a host runs `chart` on, or every reason it may not run.
///
/// A chart that declares types is type-checked first by the core checker
/// ([`check_chart`] with the CEL typer), then every CEL expression against the projected
/// [`type_env`] ([`load`] under the standard [`Policy`]); the datamodel it returns converts
/// initial values, assignments and event payloads to their declared types. A chart with any
/// error is refused, so no session exists for it. A chart without type declarations gets a
/// fresh untyped datamodel: its expressions are dynamic and are checked when first evaluated,
/// so its type errors surface at runtime as `error.execution`, exactly as before typing.
///
/// # Errors
///
/// [`LoadErrors`] listing every type error with its location in the chart.
// @tg: implements:: [[openspec:cel-datamodel#Static Type Checking of CEL]]
pub fn instantiate(chart: &Chart, limits: Limits) -> Result<CelDatamodel, LoadErrors> {
    if !is_typed(chart) {
        return Ok(CelDatamodel::new(TypeEnv::untyped(), limits));
    }
    let types = check_chart(chart, &BasicTyper::for_datamodel(CEL)).map_err(|ds| {
        LoadErrors(
            ds.into_iter()
                .map(|d| LoadError {
                    at: d.location,
                    message: d.message,
                })
                .collect(),
        )
    })?;
    let options = LoadOptions {
        env: type_env(&types),
        limits,
        policy: Policy::Standard,
    };
    load(chart, &options)
}
