//! The `statechart!` token grammar, parsed into a surface AST that keeps every token's span.
//!
//! ```text
//! chart      := attr* item*
//! attr       := '#' '[' ('deny' | 'allow') '(' ('unreachable_states' | 'unhandled_events'),* ')' ']'
//! item       := 'name' ':' (ident | str) ','?
//!             | 'datamodel' ':' ident ','?
//!             | 'binding' ':' ('early' | 'late') ','?
//!             | 'types' '{' (Name '=' type),* '}'
//!             | 'context' '{' data,* '}'
//!             | 'events' '{' (event (':' type)?),* '}'
//!             | 'initial' ident (',' ident)* ';'
//!             | state
//! state      := 'state' ident (';' | '{' body '}')
//!             | 'parallel' ident '{' body '}'
//!             | 'final' ident (';' | '{' body '}')
//!             | 'history' ident ('deep' | 'shallow')? ('=>' targets ('/' actions)?)? ';'
//! body       := ( 'initial' ident (',' ident)* ';'
//!               | 'data' '{' data,* '}'
//!               | 'entry' actions ';' | 'exit' actions ';'
//!               | 'on' events guard? ('=>' targets)? kind? ('/' actions)? ';'
//!               | 'always' guard? ('=>' targets)? kind? ('/' actions)? ';'
//!               | 'invoke' str ('src' str)? ('id' str)? 'autoforward'? ('/' actions)? ';'
//!               | state )*
//! events     := descriptor (('|' | ',') descriptor)*      descriptor: a.b.c, error.*, *, "str"
//! guard      := 'if' '!'? (ident | 'In' '(' ident ')')
//! targets    := ident (',' ident)*
//! kind       := 'internal' | 'external'
//! actions    := action (',' action)*
//! action     := ident                                     a bound Rust action function
//!             | 'raise' event
//!             | 'send' event ('to' str)? ('after' str)? ('type' str)? ('id' str)?
//!             | 'log' str
//!             | 'cancel' str
//! data       := ident (':' type)? ('=' literal)?
//! ```
//!
//! Types use the type language's textual syntax (`{ amount: decimal }`, `list<Order>`,
//! `Card { last4: string } | Cash`); initializers are Rust literals, `null`, lists `[...]` and
//! records `{ key: literal }`, written to the IR as JSON.

use proc_macro2::{Span, TokenStream, TokenTree};
use syn::ext::IdentExt;
use syn::parse::{Parse, ParseStream};
use syn::{Ident, LitStr, Token, braced, bracketed, parenthesized};

/// A name with the span of its token(s).
#[derive(Clone, Debug)]
pub struct Named {
    pub text: String,
    pub span: Span,
}

impl Named {
    fn ident(i: &Ident) -> Named {
        Named {
            text: i.unraw().to_string(),
            span: i.span(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct ChartAst {
    pub deny_unreachable: bool,
    pub deny_unhandled: bool,
    pub allow_unreachable: bool,
    pub allow_unhandled: bool,
    pub name: Option<Named>,
    pub datamodel: Option<Named>,
    pub binding: Option<Named>,
    pub types: Vec<TypeDeclAst>,
    pub context: Vec<DataAst>,
    pub events: Vec<EventAst>,
    pub initial: Vec<Named>,
    pub states: Vec<StateAst>,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct TypeDeclAst {
    pub name: Named,
    pub ty: TypeText,
}

/// Type tokens as text for the core type parser, with their span.
#[derive(Clone, Debug)]
pub struct TypeText {
    pub text: String,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct DataAst {
    pub id: Named,
    pub ty: Option<TypeText>,
    pub init: Option<TypeText>,
}

#[derive(Clone, Debug)]
pub struct EventAst {
    pub name: Named,
    pub payload: Option<TypeText>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StateKw {
    State,
    Parallel,
    Final,
    History { deep: bool },
}

#[derive(Clone, Debug)]
pub struct StateAst {
    pub kw: StateKw,
    pub id: Named,
    pub initial: Vec<Named>,
    pub data: Vec<DataAst>,
    pub entry: Vec<Vec<ActionAst>>,
    pub exit: Vec<Vec<ActionAst>>,
    pub transitions: Vec<TransitionAst>,
    pub invokes: Vec<InvokeAst>,
    pub children: Vec<StateAst>,
}

#[derive(Clone, Debug)]
pub enum GuardAst {
    Fn { name: Named, negate: bool },
    In { state: Named, negate: bool },
}

#[derive(Clone, Debug)]
pub struct TransitionAst {
    pub events: Vec<Named>,
    pub guard: Option<GuardAst>,
    pub targets: Vec<Named>,
    /// `Some(true)` for `internal`, `Some(false)` for `external`.
    pub internal: Option<bool>,
    pub actions: Vec<ActionAst>,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub enum ActionAst {
    Call(Named),
    Raise(Named),
    Send {
        event: Named,
        target: Option<String>,
        delay: Option<String>,
        type_: Option<String>,
        id: Option<String>,
        span: Span,
    },
    Log(Named),
    Cancel(Named),
}

#[derive(Clone, Debug)]
pub struct InvokeAst {
    pub type_: String,
    pub src: Option<String>,
    pub id: Option<String>,
    pub autoforward: bool,
    pub finalize: Vec<ActionAst>,
    pub span: Span,
}

fn kw(input: ParseStream<'_>, word: &str) -> bool {
    input.peek(Ident::peek_any) && input.fork().call(Ident::parse_any).is_ok_and(|i| i == word)
}

fn take_kw(input: ParseStream<'_>, word: &str) -> syn::Result<Ident> {
    let i = input.call(Ident::parse_any)?;
    if i != word {
        return Err(syn::Error::new(i.span(), format!("expected `{word}`")));
    }
    Ok(i)
}

fn eat_sep(input: ParseStream<'_>) -> syn::Result<()> {
    if input.peek(Token![,]) {
        input.parse::<Token![,]>()?;
    } else if input.peek(Token![;]) {
        input.parse::<Token![;]>()?;
    }
    Ok(())
}

/// Collect tokens up to (not including) a top-level `,` or one of `stop`, as type text.
fn type_text(input: ParseStream<'_>, stop: &[char]) -> syn::Result<TypeText> {
    let start = input.span();
    let mut ts = TokenStream::new();
    let mut last = start;
    while !input.is_empty() {
        if input.peek(Token![,]) {
            break;
        }
        let stop_here = input.step(|c| {
            if let Some((TokenTree::Punct(p), _)) = c.token_tree()
                && stop.contains(&p.as_char())
            {
                return Ok((true, *c));
            }
            Ok((false, *c))
        })?;
        if stop_here {
            break;
        }
        let tt: TokenTree = input.parse()?;
        last = tt.span();
        ts.extend([tt]);
    }
    if ts.is_empty() {
        return Err(syn::Error::new(start, "expected a type"));
    }
    Ok(TypeText {
        text: ts.to_string(),
        span: start.join(last).unwrap_or(start),
    })
}

/// An event name or descriptor: `a.b.c`, `error.*`, `*` or a string literal.
fn descriptor(input: ParseStream<'_>) -> syn::Result<Named> {
    if input.peek(LitStr) {
        let s: LitStr = input.parse()?;
        return Ok(Named {
            text: s.value(),
            span: s.span(),
        });
    }
    let start = input.span();
    let mut text = String::new();
    let mut end;
    loop {
        if input.peek(Token![*]) {
            let t: Token![*] = input.parse()?;
            end = t.span;
            text.push('*');
        } else {
            let i = input.call(Ident::parse_any).map_err(|e| {
                syn::Error::new(
                    e.span(),
                    "expected an event name such as `pay` or `error.*`",
                )
            })?;
            end = i.span();
            text.push_str(&i.unraw().to_string());
        }
        if input.peek(Token![.]) && !input.peek(Token![..]) {
            input.parse::<Token![.]>()?;
            text.push('.');
        } else {
            break;
        }
    }
    Ok(Named {
        text,
        span: start.join(end).unwrap_or(start),
    })
}

fn ident_list(input: ParseStream<'_>) -> syn::Result<Vec<Named>> {
    let mut out = vec![Named::ident(&input.call(Ident::parse_any)?)];
    while input.peek(Token![,]) {
        input.parse::<Token![,]>()?;
        out.push(Named::ident(&input.call(Ident::parse_any)?));
    }
    Ok(out)
}

fn data_list(input: ParseStream<'_>) -> syn::Result<Vec<DataAst>> {
    let content;
    braced!(content in input);
    let mut out = Vec::new();
    while !content.is_empty() {
        let id = Named::ident(&content.call(Ident::parse_any)?);
        let ty = if content.peek(Token![:]) {
            content.parse::<Token![:]>()?;
            Some(type_text(&content, &['='])?)
        } else {
            None
        };
        let init = if content.peek(Token![=]) {
            content.parse::<Token![=]>()?;
            Some(literal(&content)?)
        } else {
            None
        };
        out.push(DataAst { id, ty, init });
        if !content.is_empty() {
            content.parse::<Token![,]>()?;
        }
    }
    Ok(out)
}

/// A literal initializer, as JSON text.
fn literal(input: ParseStream<'_>) -> syn::Result<TypeText> {
    let span = input.span();
    let text = json(input)?;
    Ok(TypeText { text, span })
}

fn json(input: ParseStream<'_>) -> syn::Result<String> {
    if input.peek(Token![-]) {
        input.parse::<Token![-]>()?;
        let rest = json(input)?;
        if !rest.starts_with(|c: char| c.is_ascii_digit()) {
            return Err(input.error("expected a number after `-`"));
        }
        return Ok(format!("-{rest}"));
    }
    if input.peek(syn::token::Bracket) {
        let content;
        bracketed!(content in input);
        let mut items = Vec::new();
        while !content.is_empty() {
            items.push(json(&content)?);
            if !content.is_empty() {
                content.parse::<Token![,]>()?;
            }
        }
        return Ok(format!("[{}]", items.join(", ")));
    }
    if input.peek(syn::token::Brace) {
        let content;
        braced!(content in input);
        let mut items = Vec::new();
        while !content.is_empty() {
            let key = if content.peek(LitStr) {
                content.parse::<LitStr>()?.value()
            } else {
                content.call(Ident::parse_any)?.unraw().to_string()
            };
            content.parse::<Token![:]>()?;
            items.push(format!("{}: {}", json_string(&key), json(&content)?));
            if !content.is_empty() {
                content.parse::<Token![,]>()?;
            }
        }
        return Ok(format!("{{{}}}", items.join(", ")));
    }
    if input.peek(Ident::peek_any) {
        let i = input.call(Ident::parse_any)?;
        return match i.to_string().as_str() {
            "null" | "none" | "None" => Ok("null".to_string()),
            "true" => Ok("true".to_string()),
            "false" => Ok("false".to_string()),
            _ => Err(syn::Error::new(
                i.span(),
                "expected a literal: a number, a string, `true`, `false`, `null`, `[...]` or `{...}`",
            )),
        };
    }
    let lit: syn::Lit = input.parse().map_err(|e| {
        syn::Error::new(
            e.span(),
            "expected a literal: a number, a string, `true`, `false`, `null`, `[...]` or `{...}`",
        )
    })?;
    match lit {
        syn::Lit::Str(s) => Ok(json_string(&s.value())),
        syn::Lit::Int(i) => Ok(i.base10_digits().to_string()),
        syn::Lit::Float(f) => Ok(f.base10_digits().to_string()),
        syn::Lit::Bool(b) => Ok(b.value.to_string()),
        other => Err(syn::Error::new(
            other.span(),
            "unsupported literal in an initializer",
        )),
    }
}

/// A JSON string literal.
pub fn json_string(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn actions(input: ParseStream<'_>) -> syn::Result<Vec<ActionAst>> {
    let mut out = vec![action(input)?];
    while input.peek(Token![,]) {
        input.parse::<Token![,]>()?;
        out.push(action(input)?);
    }
    Ok(out)
}

fn lit_str(input: ParseStream<'_>, what: &str) -> syn::Result<LitStr> {
    input
        .parse::<LitStr>()
        .map_err(|e| syn::Error::new(e.span(), format!("expected {what} as a string literal")))
}

fn action(input: ParseStream<'_>) -> syn::Result<ActionAst> {
    let head = input.call(Ident::parse_any).map_err(|e| {
        syn::Error::new(
            e.span(),
            "expected an action: a function name, `raise`, `send`, `log` or `cancel`",
        )
    })?;
    match head.to_string().as_str() {
        "raise" => Ok(ActionAst::Raise(descriptor(input)?)),
        "send" => {
            let event = descriptor(input)?;
            let (mut target, mut delay, mut type_, mut id) = (None, None, None, None);
            loop {
                if kw(input, "to") {
                    take_kw(input, "to")?;
                    target = Some(lit_str(input, "the send target")?.value());
                } else if kw(input, "after") {
                    take_kw(input, "after")?;
                    delay = Some(lit_str(input, "the delay")?.value());
                } else if input.peek(Token![type]) {
                    input.parse::<Token![type]>()?;
                    type_ = Some(lit_str(input, "the send type")?.value());
                } else if kw(input, "id") {
                    take_kw(input, "id")?;
                    id = Some(lit_str(input, "the send id")?.value());
                } else {
                    break;
                }
            }
            Ok(ActionAst::Send {
                event,
                target,
                delay,
                type_,
                id,
                span: head.span(),
            })
        }
        "log" => {
            let s = lit_str(input, "the log label")?;
            Ok(ActionAst::Log(Named {
                text: s.value(),
                span: s.span(),
            }))
        }
        "cancel" => {
            let s = lit_str(input, "the send id to cancel")?;
            Ok(ActionAst::Cancel(Named {
                text: s.value(),
                span: s.span(),
            }))
        }
        _ => Ok(ActionAst::Call(Named::ident(&head))),
    }
}

fn guard(input: ParseStream<'_>) -> syn::Result<GuardAst> {
    input.parse::<Token![if]>()?;
    let negate = if input.peek(Token![!]) {
        input.parse::<Token![!]>()?;
        true
    } else {
        false
    };
    let name = input.call(Ident::parse_any).map_err(|e| {
        syn::Error::new(
            e.span(),
            "expected a guard function name or `In(state)` after `if`",
        )
    })?;
    if name == "In" && input.peek(syn::token::Paren) {
        let content;
        parenthesized!(content in input);
        let state = Named::ident(&content.call(Ident::parse_any)?);
        return Ok(GuardAst::In { state, negate });
    }
    Ok(GuardAst::Fn {
        name: Named::ident(&name),
        negate,
    })
}

/// The part of a transition after its events: guard, targets, kind and actions, then `;`.
fn transition_tail(
    input: ParseStream<'_>,
    events: Vec<Named>,
    span: Span,
) -> syn::Result<TransitionAst> {
    let guard = if input.peek(Token![if]) {
        Some(guard(input)?)
    } else {
        None
    };
    let targets = if input.peek(Token![=>]) {
        input.parse::<Token![=>]>()?;
        ident_list(input).map_err(|e| {
            syn::Error::new(
                e.span(),
                "malformed transition: expected a target state after `=>`",
            )
        })?
    } else {
        Vec::new()
    };
    let internal = if kw(input, "internal") {
        take_kw(input, "internal")?;
        Some(true)
    } else if kw(input, "external") {
        take_kw(input, "external")?;
        Some(false)
    } else {
        None
    };
    let actions = if input.peek(Token![/]) {
        input.parse::<Token![/]>()?;
        actions(input)?
    } else {
        Vec::new()
    };
    if !input.peek(Token![;]) {
        return Err(syn::Error::new(
            input.span(),
            "malformed transition: expected `if guard`, `=> target`, `internal`, `/ actions` or `;`",
        ));
    }
    input.parse::<Token![;]>()?;
    Ok(TransitionAst {
        events,
        guard,
        targets,
        internal,
        actions,
        span,
    })
}

fn is_state_kw(input: ParseStream<'_>) -> bool {
    ["state", "parallel", "final", "history"]
        .iter()
        .any(|k| kw(input, k))
}

fn state(input: ParseStream<'_>) -> syn::Result<StateAst> {
    let head = input.call(Ident::parse_any)?;
    let kw_ = match head.to_string().as_str() {
        "state" => StateKw::State,
        "parallel" => StateKw::Parallel,
        "final" => StateKw::Final,
        "history" => StateKw::History { deep: false },
        _ => return Err(syn::Error::new(head.span(), "expected a state")),
    };
    let id = Named::ident(&input.call(Ident::parse_any)?);
    let mut st = StateAst {
        kw: kw_,
        id,
        initial: Vec::new(),
        data: Vec::new(),
        entry: Vec::new(),
        exit: Vec::new(),
        transitions: Vec::new(),
        invokes: Vec::new(),
        children: Vec::new(),
    };
    if let StateKw::History { .. } = kw_ {
        let deep = if kw(input, "deep") {
            take_kw(input, "deep")?;
            true
        } else {
            if kw(input, "shallow") {
                take_kw(input, "shallow")?;
            }
            false
        };
        st.kw = StateKw::History { deep };
        if input.peek(Token![=>]) {
            let span = input.span();
            st.transitions
                .push(transition_tail(input, Vec::new(), span)?);
        } else {
            input.parse::<Token![;]>()?;
        }
        return Ok(st);
    }
    if input.peek(Token![;]) && kw_ != StateKw::Parallel {
        input.parse::<Token![;]>()?;
        return Ok(st);
    }
    let body;
    braced!(body in input);
    while !body.is_empty() {
        if is_state_kw(&body) {
            st.children.push(state(&body)?);
            continue;
        }
        let item = body.call(Ident::parse_any).map_err(|e| {
            syn::Error::new(
                e.span(),
                "expected `on`, `always`, `entry`, `exit`, `initial`, `data`, `invoke` or a state",
            )
        })?;
        match item.to_string().as_str() {
            "initial" => {
                st.initial = ident_list(&body)?;
                body.parse::<Token![;]>()?;
            }
            "data" => st.data.extend(data_list(&body)?),
            "entry" => {
                st.entry.push(actions(&body)?);
                body.parse::<Token![;]>()?;
            }
            "exit" => {
                st.exit.push(actions(&body)?);
                body.parse::<Token![;]>()?;
            }
            "on" => {
                let mut events = vec![descriptor(&body)?];
                while body.peek(Token![|]) || body.peek(Token![,]) {
                    if body.peek(Token![|]) {
                        body.parse::<Token![|]>()?;
                    } else {
                        body.parse::<Token![,]>()?;
                    }
                    events.push(descriptor(&body)?);
                }
                st.transitions
                    .push(transition_tail(&body, events, item.span())?);
            }
            "always" => st
                .transitions
                .push(transition_tail(&body, Vec::new(), item.span())?),
            "invoke" => {
                let type_ = lit_str(&body, "the invoke type")?.value();
                let mut inv = InvokeAst {
                    type_,
                    src: None,
                    id: None,
                    autoforward: false,
                    finalize: Vec::new(),
                    span: item.span(),
                };
                loop {
                    if kw(&body, "src") {
                        take_kw(&body, "src")?;
                        inv.src = Some(lit_str(&body, "the invoke source")?.value());
                    } else if kw(&body, "id") {
                        take_kw(&body, "id")?;
                        inv.id = Some(lit_str(&body, "the invoke id")?.value());
                    } else if kw(&body, "autoforward") {
                        take_kw(&body, "autoforward")?;
                        inv.autoforward = true;
                    } else {
                        break;
                    }
                }
                if body.peek(Token![/]) {
                    body.parse::<Token![/]>()?;
                    inv.finalize = actions(&body)?;
                }
                body.parse::<Token![;]>()?;
                st.invokes.push(inv);
            }
            _ => {
                return Err(syn::Error::new(
                    item.span(),
                    format!(
                        "unexpected `{item}`: expected `on`, `always`, `entry`, `exit`, \
                         `initial`, `data`, `invoke` or a state"
                    ),
                ));
            }
        }
    }
    Ok(st)
}

fn lint_attr(input: ParseStream<'_>, ast: &mut ChartAst) -> syn::Result<()> {
    input.parse::<Token![#]>()?;
    let content;
    bracketed!(content in input);
    let level: Ident = content.parse()?;
    let deny = match level.to_string().as_str() {
        "deny" => true,
        "allow" => false,
        _ => {
            return Err(syn::Error::new(
                level.span(),
                "expected `deny(...)` or `allow(...)` of `unreachable_states` or `unhandled_events`",
            ));
        }
    };
    let inner;
    parenthesized!(inner in content);
    while !inner.is_empty() {
        let i: Ident = inner.parse()?;
        match (i.to_string().as_str(), deny) {
            ("unreachable_states", true) => ast.deny_unreachable = true,
            ("unreachable_states", false) => ast.allow_unreachable = true,
            ("unhandled_events", true) => ast.deny_unhandled = true,
            ("unhandled_events", false) => ast.allow_unhandled = true,
            _ => {
                return Err(syn::Error::new(
                    i.span(),
                    "expected `unreachable_states` or `unhandled_events`",
                ));
            }
        }
        if !inner.is_empty() {
            inner.parse::<Token![,]>()?;
        }
    }
    Ok(())
}

impl Parse for ChartAst {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let mut ast = ChartAst {
            deny_unreachable: false,
            deny_unhandled: false,
            allow_unreachable: false,
            allow_unhandled: false,
            name: None,
            datamodel: None,
            binding: None,
            types: Vec::new(),
            context: Vec::new(),
            events: Vec::new(),
            initial: Vec::new(),
            states: Vec::new(),
            span: input.span(),
        };
        while input.peek(Token![#]) {
            lint_attr(input, &mut ast)?;
        }
        while !input.is_empty() {
            if is_state_kw(input) {
                ast.states.push(state(input)?);
                continue;
            }
            let key = input.call(Ident::parse_any).map_err(|e| {
                syn::Error::new(
                    e.span(),
                    "expected `name:`, `datamodel:`, `binding:`, `types`, `context`, `events`, \
                     `initial` or a state",
                )
            })?;
            match key.to_string().as_str() {
                "name" | "datamodel" | "binding" => {
                    input.parse::<Token![:]>()?;
                    let v = if input.peek(LitStr) {
                        let s: LitStr = input.parse()?;
                        Named {
                            text: s.value(),
                            span: s.span(),
                        }
                    } else {
                        Named::ident(&input.call(Ident::parse_any)?)
                    };
                    eat_sep(input)?;
                    match key.to_string().as_str() {
                        "name" => ast.name = Some(v),
                        "datamodel" => ast.datamodel = Some(v),
                        _ => ast.binding = Some(v),
                    }
                }
                "types" => {
                    let content;
                    braced!(content in input);
                    while !content.is_empty() {
                        let name = Named::ident(&content.call(Ident::parse_any)?);
                        content.parse::<Token![=]>()?;
                        let ty = type_text(&content, &[])?;
                        ast.types.push(TypeDeclAst { name, ty });
                        if !content.is_empty() {
                            content.parse::<Token![,]>()?;
                        }
                    }
                    eat_sep(input)?;
                }
                "context" => {
                    ast.context.extend(data_list(input)?);
                    eat_sep(input)?;
                }
                "events" => {
                    let content;
                    braced!(content in input);
                    while !content.is_empty() {
                        let name = descriptor(&content)?;
                        let payload = if content.peek(Token![:]) {
                            content.parse::<Token![:]>()?;
                            Some(type_text(&content, &[])?)
                        } else {
                            None
                        };
                        ast.events.push(EventAst { name, payload });
                        if !content.is_empty() {
                            content.parse::<Token![,]>()?;
                        }
                    }
                    eat_sep(input)?;
                }
                "initial" => {
                    ast.initial = ident_list(input)?;
                    input.parse::<Token![;]>()?;
                }
                other => {
                    return Err(syn::Error::new(
                        key.span(),
                        format!(
                            "unexpected `{other}`: expected `name:`, `datamodel:`, `binding:`, \
                             `types`, `context`, `events`, `initial` or a state"
                        ),
                    ));
                }
            }
        }
        Ok(ast)
    }
}
