//! Rust expressions that rebuild a chart IR value at run time (the generated `CHART`).

use proc_macro2::TokenStream;
use quote::quote;
use vertep_core::ir::{
    Binding, Block, Chart, Content, Data, DoneData, Exec, ExecKind, Extension, ExtensionValue,
    HistoryType, IfBranch, Initial, Invoke, Param, Script, Send, Span, State, StateElement,
    Transition, TransitionType,
};

fn ir() -> TokenStream {
    quote!(::vertep_dm_rust::__private::vertep_core::ir)
}

fn s(v: &str) -> TokenStream {
    quote!(::std::string::String::from(#v))
}

fn os(v: &Option<String>) -> TokenStream {
    match v {
        Some(v) => {
            let v = s(v);
            quote!(::core::option::Option::Some(#v))
        }
        None => quote!(::core::option::Option::None),
    }
}

fn strings(v: &[String]) -> TokenStream {
    let items = v.iter().map(|x| s(x));
    quote!(::std::vec![#(#items),*])
}

fn span(sp: &Span) -> TokenStream {
    let ir = ir();
    let (l, c) = (sp.line, sp.column);
    quote!(#ir::Span::new(#l, #c))
}

fn opt<T>(v: &Option<T>, f: impl Fn(&T) -> TokenStream) -> TokenStream {
    match v {
        Some(x) => {
            let t = f(x);
            quote!(::core::option::Option::Some(#t))
        }
        None => quote!(::core::option::Option::None),
    }
}

fn list<T>(v: &[T], f: impl Fn(&T) -> TokenStream) -> TokenStream {
    let items = v.iter().map(f);
    quote!(::std::vec![#(#items),*])
}

fn extension(e: &Extension) -> TokenStream {
    let ir = ir();
    let ns = s(&e.ns);
    let local = s(&e.local);
    let value = match &e.value {
        ExtensionValue::Attribute(v) => {
            let v = s(v);
            quote!(#ir::ExtensionValue::Attribute(#v))
        }
        ExtensionValue::Element(v) => {
            let v = s(v);
            quote!(#ir::ExtensionValue::Element(#v))
        }
    };
    let pos = e.position;
    let sp = span(&e.span);
    quote!(#ir::Extension { ns: #ns, local: #local, value: #value, position: #pos, span: #sp })
}

fn extensions(v: &[Extension]) -> TokenStream {
    list(v, extension)
}

fn param(p: &Param) -> TokenStream {
    let ir = ir();
    let name = s(&p.name);
    let expr = os(&p.expr);
    let location = os(&p.location);
    let sp = span(&p.span);
    quote!(#ir::Param { name: #name, expr: #expr, location: #location, span: #sp })
}

fn content(c: &Content) -> TokenStream {
    let ir = ir();
    let expr = os(&c.expr);
    let body = os(&c.body);
    let sp = span(&c.span);
    quote!(#ir::Content { expr: #expr, body: #body, span: #sp })
}

fn script(sc: &Script) -> TokenStream {
    let ir = ir();
    let src = os(&sc.src);
    let body = os(&sc.body);
    let sp = span(&sc.span);
    quote!(#ir::Script { src: #src, body: #body, span: #sp })
}

fn send(x: &Send) -> TokenStream {
    let ir = ir();
    let event = os(&x.event);
    let eventexpr = os(&x.eventexpr);
    let target = os(&x.target);
    let targetexpr = os(&x.targetexpr);
    let type_ = os(&x.type_);
    let typeexpr = os(&x.typeexpr);
    let id = os(&x.id);
    let idlocation = os(&x.idlocation);
    let delay = os(&x.delay);
    let delayexpr = os(&x.delayexpr);
    let namelist = strings(&x.namelist);
    let params = list(&x.params, param);
    let content = opt(&x.content, content);
    quote!(#ir::Send {
        event: #event, eventexpr: #eventexpr, target: #target, targetexpr: #targetexpr,
        type_: #type_, typeexpr: #typeexpr, id: #id, idlocation: #idlocation, delay: #delay,
        delayexpr: #delayexpr, namelist: #namelist, params: #params, content: #content,
    })
}

fn block(b: &Block) -> TokenStream {
    list(b, exec)
}

fn exec(e: &Exec) -> TokenStream {
    let ir = ir();
    let kind = match &e.kind {
        ExecKind::Raise { event } => {
            let event = s(event);
            quote!(#ir::ExecKind::Raise { event: #event })
        }
        ExecKind::Send(x) => {
            let x = send(x);
            quote!(#ir::ExecKind::Send(::std::boxed::Box::new(#x)))
        }
        ExecKind::Cancel { sendid, sendidexpr } => {
            let a = os(sendid);
            let b = os(sendidexpr);
            quote!(#ir::ExecKind::Cancel { sendid: #a, sendidexpr: #b })
        }
        ExecKind::Log { label, expr } => {
            let a = os(label);
            let b = os(expr);
            quote!(#ir::ExecKind::Log { label: #a, expr: #b })
        }
        ExecKind::Assign {
            location,
            expr,
            content,
        } => {
            let l = s(location);
            let x = os(expr);
            let c = os(content);
            quote!(#ir::ExecKind::Assign { location: #l, expr: #x, content: #c })
        }
        ExecKind::Script(sc) => {
            let sc = script(sc);
            quote!(#ir::ExecKind::Script(#sc))
        }
        ExecKind::If { branches } => {
            let bs = list(branches, |br: &IfBranch| {
                let cond = os(&br.cond);
                let body = block(&br.body);
                let sp = span(&br.span);
                quote!(#ir::IfBranch { cond: #cond, body: #body, span: #sp })
            });
            quote!(#ir::ExecKind::If { branches: #bs })
        }
        ExecKind::Foreach {
            array,
            item,
            index,
            body,
        } => {
            let a = s(array);
            let i = s(item);
            let ix = os(index);
            let b = block(body);
            quote!(#ir::ExecKind::Foreach { array: #a, item: #i, index: #ix, body: #b })
        }
        ExecKind::Extension(x) => {
            let x = extension(x);
            quote!(#ir::ExecKind::Extension(#x))
        }
    };
    let exts = extensions(&e.extensions);
    let sp = span(&e.span);
    quote!(#ir::Exec { kind: #kind, extensions: #exts, span: #sp })
}

fn transition(t: &Transition) -> TokenStream {
    let ir = ir();
    let events = strings(&t.events);
    let cond = os(&t.cond);
    let targets = strings(&t.targets);
    let kind = match t.kind {
        TransitionType::External => quote!(#ir::TransitionType::External),
        TransitionType::Internal => quote!(#ir::TransitionType::Internal),
    };
    let ke = t.kind_explicit;
    let body = block(&t.body);
    let exts = extensions(&t.extensions);
    let sp = span(&t.span);
    quote!(#ir::Transition {
        events: #events, cond: #cond, targets: #targets, kind: #kind, kind_explicit: #ke,
        body: #body, extensions: #exts, span: #sp,
    })
}

fn data(d: &Data) -> TokenStream {
    let ir = ir();
    let id = s(&d.id);
    let src = os(&d.src);
    let expr = os(&d.expr);
    let c = os(&d.content);
    let exts = extensions(&d.extensions);
    let sp = span(&d.span);
    quote!(#ir::Data { id: #id, src: #src, expr: #expr, content: #c, extensions: #exts, span: #sp })
}

fn invoke(i: &Invoke) -> TokenStream {
    let ir = ir();
    let type_ = os(&i.type_);
    let typeexpr = os(&i.typeexpr);
    let src = os(&i.src);
    let srcexpr = os(&i.srcexpr);
    let id = os(&i.id);
    let idlocation = os(&i.idlocation);
    let namelist = strings(&i.namelist);
    let af = i.autoforward;
    let params = list(&i.params, param);
    let c = opt(&i.content, content);
    let finalize = block(&i.finalize);
    let exts = extensions(&i.extensions);
    let sp = span(&i.span);
    quote!(#ir::Invoke {
        type_: #type_, typeexpr: #typeexpr, src: #src, srcexpr: #srcexpr, id: #id,
        idlocation: #idlocation, namelist: #namelist, autoforward: #af, params: #params,
        content: #c, finalize: #finalize, extensions: #exts, span: #sp,
    })
}

fn state(st: &State) -> TokenStream {
    let ir = ir();
    let id = s(&st.id);
    let idg = st.id_generated;
    let element = match st.element {
        StateElement::State => quote!(#ir::StateElement::State),
        StateElement::Parallel => quote!(#ir::StateElement::Parallel),
        StateElement::Final => quote!(#ir::StateElement::Final),
        StateElement::History(HistoryType::Shallow) => {
            quote!(#ir::StateElement::History(#ir::HistoryType::Shallow))
        }
        StateElement::History(HistoryType::Deep) => {
            quote!(#ir::StateElement::History(#ir::HistoryType::Deep))
        }
    };
    let initial = strings(&st.initial);
    let initial_element = opt(&st.initial_element, |i: &Initial| {
        let t = transition(&i.transition);
        let exts = extensions(&i.extensions);
        let sp = span(&i.span);
        quote!(#ir::Initial { transition: #t, extensions: #exts, span: #sp })
    });
    let children = list(&st.children, state);
    let transitions = list(&st.transitions, transition);
    let onentry = list(&st.onentry, block);
    let onexit = list(&st.onexit, block);
    let data_ = list(&st.data, data);
    let invokes = list(&st.invokes, invoke);
    let donedata = opt(&st.donedata, |d: &DoneData| {
        let c = opt(&d.content, content);
        let p = list(&d.params, param);
        let sp = span(&d.span);
        quote!(#ir::DoneData { content: #c, params: #p, span: #sp })
    });
    let exts = extensions(&st.extensions);
    let sp = span(&st.span);
    quote!(#ir::State {
        id: #id, id_generated: #idg, element: #element, initial: #initial,
        initial_element: #initial_element, children: #children, transitions: #transitions,
        onentry: #onentry, onexit: #onexit, data: #data_, invokes: #invokes,
        donedata: #donedata, extensions: #exts, span: #sp,
    })
}

/// An expression of type `vertep_core::ir::Chart` equal to `chart`, spans included.
pub fn chart(c: &Chart) -> TokenStream {
    let ir = ir();
    let name = os(&c.name);
    let dm = s(&c.datamodel);
    let dme = c.datamodel_explicit;
    let binding = match c.binding {
        Binding::Early => quote!(#ir::Binding::Early),
        Binding::Late => quote!(#ir::Binding::Late),
    };
    let be = c.binding_explicit;
    let initial = strings(&c.initial);
    let data_ = list(&c.data, data);
    let sc = opt(&c.script, script);
    let states = list(&c.states, state);
    let exts = extensions(&c.extensions);
    let sp = span(&c.span);
    quote!(#ir::Chart {
        name: #name, datamodel: #dm, datamodel_explicit: #dme, binding: #binding,
        binding_explicit: #be, initial: #initial, data: #data_, script: #sc, states: #states,
        extensions: #exts, span: #sp,
    })
}
