//! Code generation: the typed Rust API of a checked chart.
//!
//! For a chart the macros emit, into the invoking module:
//!
//! - one struct or enum per named type and per anonymous record or union, with canonical value
//!   conversions (the Rust projection of the type language);
//! - `Ctx`, the context struct, with one field per `<data>` item;
//! - `Event`, one variant per declared event carrying its payload, and `Emit`;
//! - `State`, one variant per state, with the hierarchy (`parent`, `children`, `contains`);
//! - `CHART` (the IR), `VERSION` (the chart version IRI), `NAME`;
//! - `bind()`, registering the guard and action functions the chart names under their IRIs
//!   with compiler-checked signatures, the `Statechart` marker and the `Machine` alias.

use std::collections::{BTreeMap, BTreeSet};

use proc_macro2::{Ident, Span, TokenStream};
use quote::{format_ident, quote, quote_spanned};
use vertep_core::ir::{Block, Chart, ExecKind, State, StateKind};
use vertep_core::types::{ChartTypes, Declarations, Field, Primitive, Type, Variant};

use crate::conventions::{function_name, guard_iri, is_iri};
use crate::ir_tokens;

fn dm() -> TokenStream {
    quote!(::vertep_dm_rust)
}

const KEYWORDS: &[&str] = &[
    "as", "break", "const", "continue", "crate", "else", "enum", "extern", "false", "fn", "for",
    "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut", "pub", "ref", "return",
    "self", "Self", "static", "struct", "super", "trait", "true", "type", "unsafe", "use", "where",
    "while", "async", "await", "dyn", "abstract", "become", "box", "do", "final", "macro",
    "override", "priv", "typeof", "unsized", "virtual", "yield", "try", "gen",
];

/// A Rust identifier for a name of the chart (field, data item, function).
pub fn rust_ident(name: &str, span: Span) -> Ident {
    let mut s: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if s.is_empty() || s.starts_with(|c: char| c.is_ascii_digit()) {
        s.insert(0, '_');
    }
    if matches!(s.as_str(), "self" | "Self" | "super" | "crate" | "_") {
        s.push('_');
        return Ident::new(&s, span);
    }
    if KEYWORDS.contains(&s.as_str()) {
        return Ident::new_raw(&s, span);
    }
    Ident::new(&s, span)
}

/// An UpperCamelCase identifier: `order.paid` becomes `OrderPaid`.
pub fn camel(name: &str) -> String {
    let mut out = String::new();
    for part in name.split(|c: char| !c.is_ascii_alphanumeric()) {
        let mut cs = part.chars();
        if let Some(f) = cs.next() {
            out.extend(f.to_uppercase());
            out.push_str(cs.as_str());
        }
    }
    if out.is_empty() || out.starts_with(|c: char| c.is_ascii_digit()) {
        out.insert(0, 'V');
    }
    if matches!(out.as_str(), "Self") {
        out.push('_');
    }
    out
}

fn doc(text: &str) -> TokenStream {
    quote!(#[doc = #text])
}

/// The Rust projection of the chart's types.
pub struct TypeGen<'a> {
    decls: &'a Declarations,
    recursive: BTreeSet<String>,
    taken: BTreeSet<String>,
    pub items: Vec<TokenStream>,
    pub errors: Vec<syn::Error>,
    span: Span,
}

impl<'a> TypeGen<'a> {
    pub fn new(decls: &'a Declarations, span: Span) -> Self {
        let mut recursive = BTreeSet::new();
        for d in &decls.types {
            // `d` is recursive when it reaches itself through references.
            let mut seen = BTreeSet::new();
            let mut stack: Vec<&str> = d.ty.references();
            while let Some(n) = stack.pop() {
                if n == d.name {
                    recursive.insert(d.name.clone());
                    break;
                }
                if seen.insert(n)
                    && let Some(t) = decls.lookup(n)
                {
                    stack.extend(t.references());
                }
            }
        }
        let taken = decls.types.iter().map(|d| d.name.clone()).collect();
        TypeGen {
            decls,
            recursive,
            taken,
            items: Vec::new(),
            errors: Vec::new(),
            span,
        }
    }

    fn fresh(&mut self, hint: &str) -> String {
        let base = camel(hint);
        let mut name = base.clone();
        let mut n = 2;
        while self.taken.contains(&name) {
            name = format!("{base}{n}");
            n += 1;
        }
        self.taken.insert(name.clone());
        name
    }

    fn needs_box(&self, t: &Type) -> bool {
        t.references().iter().any(|n| self.recursive.contains(*n))
    }

    /// The Rust type of `t`; anonymous records and unions become new items named after `hint`.
    pub fn rust_ty(&mut self, t: &Type, hint: &str) -> TokenStream {
        let dm = dm();
        match t {
            Type::Primitive(p) => match p {
                Primitive::Bool => quote!(bool),
                Primitive::Int => quote!(i64),
                Primitive::Decimal => quote!(#dm::Decimal),
                Primitive::String => quote!(::std::string::String),
                Primitive::Datetime => quote!(#dm::Datetime),
                Primitive::Iri => quote!(#dm::Iri),
            },
            Type::Value => quote!(#dm::Value),
            Type::List(inner) => {
                let i = self.rust_ty(inner, &format!("{hint}Item"));
                quote!(::std::vec::Vec<#i>)
            }
            Type::Optional(inner) => {
                let i = self.rust_ty(inner, hint);
                if self.needs_box(inner) {
                    quote!(::core::option::Option<::std::boxed::Box<#i>>)
                } else {
                    quote!(::core::option::Option<#i>)
                }
            }
            Type::Named(n) => {
                let i = Ident::new(n, self.span);
                quote!(#i)
            }
            Type::Record(fields) => {
                let name = self.fresh(hint);
                self.record(&name, fields, &format!("The record type `{t}`."));
                let i = Ident::new(&name, self.span);
                quote!(#i)
            }
            Type::Union(vs) => {
                let name = self.fresh(hint);
                self.union(&name, vs, &format!("The union type `{t}`."));
                let i = Ident::new(&name, self.span);
                quote!(#i)
            }
        }
    }

    /// Field identifiers and types of a record.
    pub fn fields(&mut self, owner: &str, fields: &[Field]) -> Vec<(Ident, TokenStream, String)> {
        let mut used = BTreeSet::new();
        let mut out = Vec::new();
        for f in fields {
            let mut id = rust_ident(&f.name, self.span);
            let mut n = 2;
            while !used.insert(id.to_string()) {
                id = rust_ident(&format!("{}_{n}", f.name), self.span);
                n += 1;
            }
            let ty = self.rust_ty(&f.ty, &format!("{owner}{}", camel(&f.name)));
            out.push((id, ty, f.name.clone()));
        }
        out
    }

    fn record(&mut self, name: &str, fields: &[Field], about: &str) {
        let dm = dm();
        let ident = Ident::new(name, self.span);
        let fs = self.fields(name, fields);
        let ids: Vec<&Ident> = fs.iter().map(|f| &f.0).collect();
        let tys: Vec<&TokenStream> = fs.iter().map(|f| &f.1).collect();
        let wires: Vec<&String> = fs.iter().map(|f| &f.2).collect();
        let docs: Vec<TokenStream> = fs
            .iter()
            .map(|f| doc(&format!("Field `{}`.", f.2)))
            .collect();
        let about = doc(about);
        self.items.push(quote! {
            #about
            #[derive(Clone, Debug, PartialEq)]
            pub struct #ident { #( #docs pub #ids: #tys, )* }

            impl ::core::default::Default for #ident {
                fn default() -> Self {
                    #ident { #( #ids: ::core::default::Default::default(), )* }
                }
            }

            impl #dm::FromValue for #ident {
                fn from_value(value: &#dm::Value) -> ::core::result::Result<Self, #dm::EvalError> {
                    let entries = #dm::value::record(value, &[#(#wires),*])?;
                    ::core::result::Result::Ok(#ident { #( #ids: #dm::value::field(entries, #wires)?, )* })
                }
            }

            impl #dm::ToValue for #ident {
                fn to_value(&self) -> #dm::Value {
                    #dm::value::make_record(::std::vec![#( (#wires, &self.#ids as &dyn #dm::ToValue) ),*])
                }
            }
        });
    }

    fn union(&mut self, name: &str, variants: &[Variant], about: &str) {
        let dm = dm();
        let ident = Ident::new(name, self.span);
        let mut used = BTreeSet::new();
        let mut defs = Vec::new();
        let mut from = Vec::new();
        let mut to = Vec::new();
        let mut default = None;
        for v in variants {
            let vn = camel(&v.tag);
            if !used.insert(vn.clone()) {
                self.errors.push(syn::Error::new(
                    self.span,
                    format!(
                        "union `{name}`: tags `{}` map to the same Rust variant `{vn}`",
                        v.tag
                    ),
                ));
                continue;
            }
            let vi = Ident::new(&vn, self.span);
            let tag = &v.tag;
            let vdoc = doc(&format!("Variant `{tag}`."));
            match &v.payload {
                // An inline record payload becomes a struct variant: `Card { last4: String }`.
                Some(Type::Record(fields)) => {
                    let fs = self.fields(&format!("{name}{vn}"), fields);
                    let ids: Vec<&Ident> = fs.iter().map(|f| &f.0).collect();
                    let tys: Vec<&TokenStream> = fs.iter().map(|f| &f.1).collect();
                    let wires: Vec<&String> = fs.iter().map(|f| &f.2).collect();
                    let fdocs: Vec<TokenStream> = fs
                        .iter()
                        .map(|f| doc(&format!("Field `{}`.", f.2)))
                        .collect();
                    defs.push(quote!(#vdoc #vi { #( #fdocs #ids: #tys ),* }));
                    from.push(quote!(#tag => {
                        let p = payload.ok_or_else(|| #dm::EvalError::new(::std::format!("variant `{}` requires a payload (`value`)", #tag)))?;
                        let entries = #dm::value::record(p, &[#(#wires),*])?;
                        ::core::result::Result::Ok(#ident::#vi { #( #ids: #dm::value::field(entries, #wires)?, )* })
                    }));
                    to.push(quote!(#ident::#vi { #(#ids),* } => #dm::value::make_union(#tag, ::core::option::Option::Some(
                        #dm::value::make_record(::std::vec![#( (#wires, #ids as &dyn #dm::ToValue) ),*])
                    ))));
                    default.get_or_insert(
                        quote!(#ident::#vi { #( #ids: ::core::default::Default::default(), )* }),
                    );
                }
                Some(p) => {
                    let ty = self.rust_ty(p, &format!("{name}{vn}"));
                    defs.push(quote!(#vdoc #vi(#ty)));
                    from.push(quote!(#tag => ::core::result::Result::Ok(#ident::#vi(#dm::value::payload(tag, payload)?))));
                    to.push(quote!(#ident::#vi(x) => #dm::value::make_union(#tag, ::core::option::Option::Some(#dm::ToValue::to_value(x)))));
                    default.get_or_insert(quote!(#ident::#vi(::core::default::Default::default())));
                }
                None => {
                    defs.push(quote!(#vdoc #vi));
                    from.push(quote!(#tag => match payload {
                        ::core::option::Option::None => ::core::result::Result::Ok(#ident::#vi),
                        ::core::option::Option::Some(_) => ::core::result::Result::Err(#dm::EvalError::new(::std::format!("variant `{}` takes no payload", #tag))),
                    }));
                    to.push(quote!(#ident::#vi => #dm::value::make_union(#tag, ::core::option::Option::None)));
                    default.get_or_insert(quote!(#ident::#vi));
                }
            }
        }
        let default = default.unwrap_or_else(|| quote!(unreachable!()));
        let about = doc(about);
        self.items.push(quote! {
            #about
            #[derive(Clone, Debug, PartialEq)]
            pub enum #ident { #(#defs),* }

            impl ::core::default::Default for #ident {
                fn default() -> Self { #default }
            }

            impl #dm::FromValue for #ident {
                fn from_value(value: &#dm::Value) -> ::core::result::Result<Self, #dm::EvalError> {
                    let (tag, payload) = #dm::value::union_parts(value)?;
                    match tag {
                        #(#from,)*
                        other => ::core::result::Result::Err(#dm::value::unknown_tag(other)),
                    }
                }
            }

            impl #dm::ToValue for #ident {
                fn to_value(&self) -> #dm::Value {
                    match self { #(#to,)* }
                }
            }
        });
    }

    /// Items for every named type.
    pub fn named(&mut self) {
        let dm = dm();
        let _ = &dm;
        for d in &self.decls.types {
            let about = format!("The chart type `{}` = `{}`.", d.name, d.ty);
            match &d.ty {
                Type::Record(fs) => self.record(&d.name, fs, &about),
                Type::Union(vs) => self.union(&d.name, vs, &about),
                other => {
                    let ident = Ident::new(&d.name, self.span);
                    let ty = self.rust_ty(other, &d.name);
                    let about = doc(&about);
                    self.items.push(quote!(#about pub type #ident = #ty;));
                }
            }
        }
    }

    /// The structural type of `t` after following named types.
    pub fn resolve<'t>(&'t self, mut t: &'t Type) -> &'t Type {
        for _ in 0..=self.decls.types.len() {
            match t {
                Type::Named(n) => match self.decls.lookup(n) {
                    Some(x) => t = x,
                    None => return t,
                },
                other => return other,
            }
        }
        t
    }
}

/// How a guard or action is used.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Role {
    Guard,
    Action,
}

/// The event argument a guard or action receives where it is used.
fn arg_for_events(
    events: Option<&[String]>,
    payloads: &BTreeMap<String, TokenStream>,
) -> TokenStream {
    let dm = dm();
    match events {
        None => quote!(#dm::RawEvent),
        Some([]) => quote!(()),
        Some([d]) => match payloads.get(d.trim()) {
            Some(p) => p.clone(),
            None => quote!(#dm::RawEvent),
        },
        Some(_) => quote!(#dm::RawEvent),
    }
}

struct FnUse {
    iri: String,
    role: Role,
    arg: TokenStream,
}

fn collect_uses(chart: &Chart, payloads: &BTreeMap<String, TokenStream>) -> Vec<FnUse> {
    fn block(
        b: &Block,
        events: Option<&[String]>,
        p: &BTreeMap<String, TokenStream>,
        out: &mut Vec<FnUse>,
    ) {
        for e in b {
            match &e.kind {
                ExecKind::Foreach { array, body, .. } => {
                    if is_iri(array) {
                        out.push(FnUse {
                            iri: array.trim().to_string(),
                            role: Role::Action,
                            arg: arg_for_events(events, p),
                        });
                    }
                    block(body, events, p, out);
                }
                ExecKind::If { branches } => {
                    for br in branches {
                        if let Some(iri) = br.cond.as_deref().and_then(guard_iri) {
                            out.push(FnUse {
                                iri: iri.to_string(),
                                role: Role::Guard,
                                arg: arg_for_events(events, p),
                            });
                        }
                        block(&br.body, events, p, out);
                    }
                }
                _ => {}
            }
        }
    }
    fn state(s: &State, p: &BTreeMap<String, TokenStream>, out: &mut Vec<FnUse>) {
        for b in s.onentry.iter().chain(&s.onexit) {
            block(b, None, p, out);
        }
        if let Some(i) = &s.initial_element {
            block(&i.transition.body, Some(&[]), p, out);
        }
        for t in &s.transitions {
            if let Some(iri) = t.cond.as_deref().and_then(guard_iri) {
                out.push(FnUse {
                    iri: iri.to_string(),
                    role: Role::Guard,
                    arg: arg_for_events(Some(&t.events), p),
                });
            }
            block(&t.body, Some(&t.events), p, out);
        }
        for inv in &s.invokes {
            block(&inv.finalize, None, p, out);
        }
        for c in &s.children {
            state(c, p, out);
        }
    }
    let mut out = Vec::new();
    for s in &chart.states {
        state(s, payloads, &mut out);
    }
    out
}

/// What the generator needs besides the chart.
pub struct Extra<'a> {
    /// Token spans of function names (statechart!); others use `call`.
    pub functions: &'a BTreeMap<String, Span>,
    /// Warnings to emit, with their spans.
    pub warnings: &'a [(Span, String)],
    /// The included file, tracked for rebuilds.
    pub tracked: Option<String>,
    /// The macro call site.
    pub call: Span,
}

/// Generate the typed API. The chart must have passed every check.
// @tg: implements:: [[openspec:rust-dsl#Generated Typed API]]
// @tg: implements:: [[openspec:type-system#Rust Projection]]
// @tg: implements:: [[openspec:rust-dsl#Guard and Action Signature Checking]]
pub fn generate(chart: &Chart, types: &ChartTypes, extra: &Extra<'_>) -> TokenStream {
    let dm = dm();
    let call = extra.call;
    let mut tg = TypeGen::new(&types.declarations, call);
    tg.named();

    // Events.
    let mut ev_defs = Vec::new();
    let mut ev_names = Vec::new();
    let mut ev_data = Vec::new();
    let mut ev_from = Vec::new();
    let mut payloads: BTreeMap<String, TokenStream> = BTreeMap::new();
    let mut used_variants = BTreeSet::new();
    for (name, payload) in types.events.iter() {
        let vn = camel(name);
        if !used_variants.insert(vn.clone()) {
            tg.errors.push(syn::Error::new(
                call,
                format!("event `{name}` maps to the Rust variant `Event::{vn}` of another event"),
            ));
            continue;
        }
        let vi = Ident::new(&vn, call);
        let vdoc = doc(&format!("Event `{name}`."));
        ev_names.push(quote!(Event::#vi { .. } => #name));
        match payload {
            None => {
                ev_defs.push(quote!(#vdoc #vi));
                ev_data.push(quote!(Event::#vi => #dm::Value::Undefined));
                ev_from.push(quote!(#name => ::core::result::Result::Ok(Event::#vi)));
                payloads.insert(name.to_string(), quote!(()));
            }
            Some(t) => {
                let resolved = tg.resolve(t).clone();
                if let Type::Record(fields) = &resolved {
                    let fs = tg.fields(&format!("{vn}Payload"), fields);
                    let ids: Vec<&Ident> = fs.iter().map(|f| &f.0).collect();
                    let tys: Vec<&TokenStream> = fs.iter().map(|f| &f.1).collect();
                    let wires: Vec<&String> = fs.iter().map(|f| &f.2).collect();
                    let fdocs: Vec<TokenStream> = fs
                        .iter()
                        .map(|f| doc(&format!("Field `{}`.", f.2)))
                        .collect();
                    ev_defs.push(quote!(#vdoc #vi { #( #fdocs #ids: #tys ),* }));
                    ev_data.push(quote!(Event::#vi { #(ref #ids),* } => #dm::value::make_record(::std::vec![#( (#wires, #ids as &dyn #dm::ToValue) ),*])));
                    ev_from.push(quote!(#name => {
                        let entries = #dm::value::record(&event.data, &[#(#wires),*])?;
                        ::core::result::Result::Ok(Event::#vi { #( #ids: #dm::value::field(entries, #wires)?, )* })
                    }));
                    // The payload type a guard or action receives.
                    let p = match t {
                        Type::Named(_) => tg.rust_ty(t, name),
                        _ => tg.rust_ty(t, &format!("{vn}Payload")),
                    };
                    payloads.insert(name.to_string(), p);
                } else {
                    let ty = tg.rust_ty(t, &format!("{vn}Payload"));
                    ev_defs.push(quote!(#vdoc #vi(#ty)));
                    ev_data.push(quote!(Event::#vi(ref x) => #dm::ToValue::to_value(x)));
                    ev_from.push(quote!(#name => ::core::result::Result::Ok(Event::#vi(<#ty as #dm::FromValue>::from_value(&event.data)?))));
                    payloads.insert(name.to_string(), ty);
                }
            }
        }
    }

    // Context.
    let ctx_fields = tg.fields("Ctx", &types.context);
    let c_ids: Vec<&Ident> = ctx_fields.iter().map(|f| &f.0).collect();
    let c_tys: Vec<&TokenStream> = ctx_fields.iter().map(|f| &f.1).collect();
    let c_wires: Vec<&String> = ctx_fields.iter().map(|f| &f.2).collect();
    let c_docs: Vec<TokenStream> = types
        .context
        .iter()
        .map(|f| doc(&format!("Data item `{}` of type `{}`.", f.name, f.ty)))
        .collect();

    // States.
    let mut flat: Vec<(&State, Option<usize>)> = Vec::new();
    fn walk<'c>(
        states: &'c [State],
        parent: Option<usize>,
        out: &mut Vec<(&'c State, Option<usize>)>,
    ) {
        for s in states {
            let i = out.len();
            out.push((s, parent));
            walk(&s.children, Some(i), out);
        }
    }
    walk(&chart.states, None, &mut flat);
    let mut s_vars = Vec::new();
    let mut used_states = BTreeSet::new();
    for (s, _) in &flat {
        let vn = camel(&s.id);
        if !used_states.insert(vn.clone()) {
            tg.errors.push(syn::Error::new(
                call,
                format!(
                    "state `{}` maps to the Rust variant `State::{vn}` of another state",
                    s.id
                ),
            ));
        }
        s_vars.push(Ident::new(&vn, call));
    }
    let s_ids: Vec<&str> = flat.iter().map(|(s, _)| s.id.as_str()).collect();
    let s_docs: Vec<TokenStream> = flat
        .iter()
        .map(|(s, _)| doc(&format!("State `{}`.", s.id)))
        .collect();
    let ir = quote!(#dm::__private::vertep_core::ir);
    let s_kinds: Vec<TokenStream> = flat
        .iter()
        .map(|(s, _)| match s.kind() {
            StateKind::Atomic => quote!(#ir::StateKind::Atomic),
            StateKind::Compound => quote!(#ir::StateKind::Compound),
            StateKind::Parallel => quote!(#ir::StateKind::Parallel),
            StateKind::Final => quote!(#ir::StateKind::Final),
            StateKind::History(vertep_core::ir::HistoryType::Shallow) => {
                quote!(#ir::StateKind::History(#ir::HistoryType::Shallow))
            }
            StateKind::History(vertep_core::ir::HistoryType::Deep) => {
                quote!(#ir::StateKind::History(#ir::HistoryType::Deep))
            }
        })
        .collect();
    let s_parents: Vec<TokenStream> = flat
        .iter()
        .map(|(_, p)| match p {
            Some(p) => {
                let v = &s_vars[*p];
                quote!(::core::option::Option::Some(State::#v))
            }
            None => quote!(::core::option::Option::None),
        })
        .collect();
    let s_children: Vec<TokenStream> = (0..flat.len())
        .map(|i| {
            let kids = flat
                .iter()
                .enumerate()
                .filter(|(_, (_, p))| *p == Some(i))
                .map(|(k, _)| &s_vars[k]);
            quote!(&[#(State::#kids),*])
        })
        .collect();

    // Bindings.
    let uses = collect_uses(chart, &payloads);
    let mut bound: BTreeMap<(String, bool), String> = BTreeMap::new();
    let mut binds = Vec::new();
    for u in &uses {
        let name = function_name(&u.iri).to_string();
        let is_guard = u.role == Role::Guard;
        let arg_text = u.arg.to_string();
        match bound.get(&(u.iri.clone(), is_guard)) {
            Some(prev) if *prev == arg_text => continue,
            Some(prev) => {
                let span = extra.functions.get(&name).copied().unwrap_or(call);
                tg.errors.push(syn::Error::new(
                    span,
                    format!(
                        "`{name}` is used where events carry `{}` and where they carry `{}`; one \
                         function has one event argument type",
                        prev.replace(' ', ""),
                        arg_text.replace(' ', "")
                    ),
                ));
                continue;
            }
            None => {
                bound.insert((u.iri.clone(), is_guard), arg_text);
            }
        }
        let span = extra.functions.get(&name).copied().unwrap_or(call);
        if !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
            || name.starts_with(|c: char| c.is_ascii_digit())
        {
            tg.errors.push(syn::Error::new(
                span,
                format!(
                    "`{}` does not end in a Rust function name; bind it with `Registry` instead",
                    u.iri
                ),
            ));
            continue;
        }
        let f = rust_ident(&name, span);
        let iri = &u.iri;
        let arg = &u.arg;
        binds.push(if is_guard {
            quote_spanned!(span=> registry.guard::<#arg, _>(#iri, #f).expect("statechart! guard IRIs are unique");)
        } else {
            quote_spanned!(span=> registry.action::<#arg, Event, _, _>(#iri, #f).expect("statechart! action IRIs are unique");)
        });
    }

    // Warnings, as deprecation notes on generated functions used at the offending tokens.
    let warnings: Vec<TokenStream> = extra
        .warnings
        .iter()
        .enumerate()
        .map(|(i, (span, msg))| {
            let f = format_ident!("vertep_warning_{}", i, span = *span);
            let note = format!("vertep: {msg}");
            quote_spanned! {*span=>
                #[deprecated(note = #note)]
                fn #f() {}
                #f();
            }
        })
        .collect();
    let warnings = if warnings.is_empty() {
        quote!()
    } else {
        quote! {
            #[allow(dead_code, non_snake_case)]
            fn __vertep_chart_warnings() { #(#warnings)* }
        }
    };
    let tracked = match &extra.tracked {
        Some(path) => quote!(
            const _: &[u8] = ::core::include_bytes!(#path);
        ),
        None => quote!(),
    };

    if !tg.errors.is_empty() {
        let mut errs = tg.errors.into_iter();
        let mut first = errs
            .next()
            .unwrap_or_else(|| syn::Error::new(call, "codegen error"));
        for e in errs {
            first.combine(e);
        }
        return first.to_compile_error();
    }
    let type_items = tg.items;
    let chart_expr = ir_tokens::chart(chart);
    let name = chart.name.clone().unwrap_or_default();
    let declare_arm = if c_wires.is_empty() {
        quote!()
    } else {
        quote!(#(#c_wires)|* => ::core::result::Result::Ok(()),)
    };
    let ctx_doc = doc(&format!(
        "The context of chart `{name}`: one field per data item."
    ));
    let state_doc = doc(&format!("The states of chart `{name}` in document order."));
    let event_doc = doc(&format!(
        "The declared events of chart `{name}`, with their payloads."
    ));

    quote! {
        #(#type_items)*

        #ctx_doc
        #[derive(Clone, Debug, PartialEq)]
        pub struct Ctx { #( #c_docs pub #c_ids: #c_tys, )* }

        impl ::core::default::Default for Ctx {
            fn default() -> Self { Ctx { #( #c_ids: ::core::default::Default::default(), )* } }
        }

        impl #dm::Context for Ctx {
            fn declare(&mut self, field: &str) -> ::core::result::Result<(), #dm::EvalError> {
                match field {
                    #declare_arm
                    other => ::core::result::Result::Err(#dm::EvalError::new(::std::format!("`{}` is not a field of Ctx", other))),
                }
            }
            fn get(&self, field: &str) -> ::core::option::Option<#dm::Value> {
                match field {
                    #( #c_wires => ::core::option::Option::Some(#dm::ToValue::to_value(&self.#c_ids)), )*
                    _ => ::core::option::Option::None,
                }
            }
            fn set(&mut self, field: &str, value: &#dm::Value) -> ::core::result::Result<(), #dm::EvalError> {
                match field {
                    #( #c_wires => {
                        self.#c_ids = #dm::FromValue::from_value(value).map_err(|e| #dm::EvalError::new(::std::format!("ctx.{}: {}", #c_wires, e.message)))?;
                        ::core::result::Result::Ok(())
                    } )*
                    other => ::core::result::Result::Err(#dm::EvalError::new(::std::format!("`{}` is not a field of Ctx", other))),
                }
            }
            fn snapshot(&self) -> #dm::Value {
                #dm::value::make_record(::std::vec![#( (#c_wires, &self.#c_ids as &dyn #dm::ToValue) ),*])
            }
            fn restore(value: &#dm::Value) -> ::core::result::Result<Self, #dm::EvalError> {
                let entries = #dm::value::record(value, &[#(#c_wires),*])?;
                ::core::result::Result::Ok(Ctx { #( #c_ids: #dm::value::field(entries, #c_wires)?, )* })
            }
        }

        #event_doc
        #[derive(Clone, Debug, PartialEq)]
        pub enum Event { #(#ev_defs),* }

        impl #dm::ChartEvent for Event {
            fn name(&self) -> &'static str {
                match *self { #(#ev_names,)* }
            }
            fn data(&self) -> #dm::Value {
                match *self { #(#ev_data,)* }
            }
            fn from_event(event: &#dm::RawEvent) -> ::core::result::Result<Self, #dm::EvalError> {
                match event.name.as_str() {
                    #(#ev_from,)*
                    other => ::core::result::Result::Err(#dm::EvalError::new(::std::format!("`{}` is not a declared event", other))),
                }
            }
        }

        /// The emitter actions of this chart receive.
        pub type Emit = #dm::Emit<Event>;

        #state_doc
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub enum State { #( #s_docs #s_vars, )* }

        impl #dm::StateEnum for State {
            const ALL: &'static [Self] = &[#(State::#s_vars),*];
            fn index(self) -> usize { self as usize }
            fn id(self) -> &'static str { match self { #( State::#s_vars => #s_ids, )* } }
            fn kind(self) -> #ir::StateKind { match self { #( State::#s_vars => #s_kinds, )* } }
            fn parent(self) -> ::core::option::Option<Self> { match self { #( State::#s_vars => #s_parents, )* } }
            fn children(self) -> &'static [Self] { match self { #( State::#s_vars => #s_children, )* } }
        }

        impl State {
            /// The state identifier.
            pub fn id(self) -> &'static str { <Self as #dm::StateEnum>::id(self) }
            /// The parent state.
            pub fn parent(self) -> ::core::option::Option<Self> { <Self as #dm::StateEnum>::parent(self) }
            /// The child states in document order.
            pub fn children(self) -> &'static [Self] { <Self as #dm::StateEnum>::children(self) }
            /// Whether `self` is `other` or an ancestor of it.
            pub fn contains(self, other: Self) -> bool { <Self as #dm::StateEnum>::contains(self, other) }
        }

        /// The chart's name.
        pub const NAME: &str = #name;

        /// The chart IR, as `statechart!` or `include_chart!` built and checked it.
        pub static CHART: #dm::__private::LazyLock<#ir::Chart> =
            #dm::__private::LazyLock::new(|| #chart_expr);

        /// The chart version IRI (`urn:vertep:chart:sha256:...`), equal to what `vt hash`
        /// computes for the chart.
        pub static VERSION: #dm::__private::LazyLock<::std::string::String> =
            #dm::__private::LazyLock::new(|| #dm::version::chart_version_iri(&CHART));

        /// A registry with every guard and action of the chart bound to the Rust function of
        /// the same name, under the chart's IRIs.
        pub fn bind() -> #dm::Registry<Ctx> {
            let mut registry = #dm::Registry::<Ctx>::new();
            #(#binds)*
            registry
        }

        /// The chart's types and functions, for [`Machine`].
        #[derive(Clone, Copy, Debug, Default)]
        pub struct Statechart;

        impl #dm::ChartSpec for Statechart {
            type Ctx = Ctx;
            type State = State;
            type Event = Event;
            fn chart() -> &'static #ir::Chart { &CHART }
            fn compiled() -> &'static #dm::__private::vertep_core::compile::CompiledChart {
                static COMPILED: #dm::__private::LazyLock<#dm::__private::vertep_core::compile::CompiledChart> =
                    #dm::__private::LazyLock::new(|| {
                        #dm::__private::vertep_core::compile::CompiledChart::new(CHART.clone())
                            .expect("the chart was validated at compile time")
                    });
                &COMPILED
            }
            fn bind() -> #dm::Registry<Ctx> { bind() }
        }

        /// A running session of this chart.
        pub type Machine = #dm::Machine<Statechart>;

        #warnings
        #tracked
    }
}
