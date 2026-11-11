//! Static checks on chart-supplied ECMAScript text, done with boa's parser before anything
//! is compiled, so that an expression or location can never escape the code it is spliced into.

use boa_engine::Source;
use boa_engine::ast::StatementListItem;
use boa_engine::ast::declaration::{Binding, Declaration, LexicalDeclaration, VariableList};
use boa_engine::ast::expression::Expression;
use boa_engine::ast::expression::access::PropertyAccess;
use boa_engine::ast::operations::bound_names;
use boa_engine::ast::statement::Statement;
use boa_engine::interner::{Interner, ToInternedString};
use boa_engine::parser::Parser;
use vertep_core::datamodel::EvalError;

/// The shape of a legal assignment location.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Location {
    /// A plain variable, such as `count`.
    Variable(String),
    /// A property of an object, such as `a.b` or `a["b"]`.
    Member {
        /// The leftmost identifier, when the base is a chain of property accesses on one.
        root: Option<String>,
    },
}

fn parse_items(text: &str, interner: &mut Interner) -> Result<boa_engine::ast::Script, EvalError> {
    Parser::new(Source::from_bytes(text))
        .parse_eval(false, interner)
        .map(|output| output.0)
        .map_err(|e| EvalError::new(format!("SyntaxError: {e}")))
}

/// Parses `text` as one complete ECMAScript expression and returns it with its interner.
fn parse_expression(text: &str) -> Result<(Expression, Interner), EvalError> {
    let mut interner = Interner::default();
    // The trailing newline ends a line comment the text may finish with.
    let script = parse_items(&format!("({text}\n)"), &mut interner)?;
    if let [StatementListItem::Statement(statement)] = &script.statements()[..]
        && let Statement::Expression(Expression::Parenthesized(inner)) = statement.as_ref()
    {
        return Ok((inner.expression().clone(), interner));
    }
    Err(EvalError::new(format!(
        "SyntaxError: `{text}` is not a single expression"
    )))
}

/// `text` without trailing whitespace and statement terminators: `new f();` is accepted as the
/// expression `new f()`, as charts written for statement-evaluating engines expect (IRP test
/// 452). Removing a final `;` never changes what a valid expression means, and the result is
/// still checked with [`check_expression`].
pub(crate) fn strip_terminator(text: &str) -> &str {
    text.trim_end().trim_end_matches(';').trim_end()
}

/// Checks that `text` is exactly one expression.
pub(crate) fn check_expression(text: &str) -> Result<(), EvalError> {
    parse_expression(text).map(|_| ())
}

fn root_identifier(expr: &Expression, interner: &Interner) -> Option<String> {
    match expr {
        Expression::Identifier(id) => Some(interner.resolve_expect(id.sym()).to_string()),
        Expression::PropertyAccess(PropertyAccess::Simple(access)) => {
            root_identifier(access.target(), interner)
        }
        Expression::Parenthesized(p) => root_identifier(p.expression(), interner),
        _ => None,
    }
}

/// Parses `text` as a legal assignment location: a variable or a property access.
pub(crate) fn parse_location(text: &str) -> Result<Location, EvalError> {
    let (expr, interner) = parse_expression(text)?;
    match &expr {
        Expression::Identifier(id) => {
            let name = interner.resolve_expect(id.sym()).to_string();
            // Escaped identifiers (`a`) are legal ECMAScript but not legal data ids.
            if name == text.trim() {
                Ok(Location::Variable(name))
            } else {
                Err(EvalError::new(format!(
                    "`{text}` is not a legal location: escaped identifiers are not allowed"
                )))
            }
        }
        Expression::PropertyAccess(PropertyAccess::Simple(access)) => Ok(Location::Member {
            root: root_identifier(access.target(), &interner),
        }),
        _ => Err(EvalError::new(format!("`{text}` is not a legal location"))),
    }
}

/// Parses `text` as a plain variable name.
pub(crate) fn parse_variable(text: &str) -> Result<String, EvalError> {
    match parse_location(text)? {
        Location::Variable(name) => Ok(name),
        Location::Member { .. } => Err(EvalError::new(format!(
            "`{text}` is not a legal variable name"
        ))),
    }
}

/// What a script declares at its top level that the snapshot must know about.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct ScriptBindings {
    /// Global lexical bindings (`let`, `const`, `class`), with whether each is `const`.
    pub(crate) lexical: Vec<(String, bool)>,
    /// Classes bound to a top-level name, with their source text. boa does not keep the
    /// source text of class constructors, so it is regenerated from the syntax tree.
    pub(crate) classes: Vec<(String, String)>,
}

fn class_initializers(list: &VariableList, interner: &Interner, out: &mut Vec<(String, String)>) {
    for variable in list.as_ref() {
        if let (Binding::Identifier(id), Some(Expression::ClassExpression(class))) =
            (variable.binding(), variable.init())
        {
            out.push((
                interner.resolve_expect(id.sym()).to_string(),
                class.to_interned_string(interner),
            ));
        }
    }
}

/// The top-level bindings a script declares. A script that does not parse declares nothing.
pub(crate) fn script_bindings(script: &str) -> ScriptBindings {
    let mut interner = Interner::default();
    let Ok(parsed) = parse_items(script, &mut interner) else {
        return ScriptBindings::default();
    };
    let mut out = ScriptBindings::default();
    for item in parsed.statements().iter() {
        match item {
            StatementListItem::Declaration(declaration) => match declaration.as_ref() {
                Declaration::Lexical(lexical) => {
                    let is_const = matches!(lexical, LexicalDeclaration::Const(_));
                    for sym in bound_names(lexical) {
                        out.lexical
                            .push((interner.resolve_expect(sym).to_string(), is_const));
                    }
                    class_initializers(lexical.variable_list(), &interner, &mut out.classes);
                }
                Declaration::ClassDeclaration(class) => {
                    let name = interner.resolve_expect(class.name().sym()).to_string();
                    out.lexical.push((name.clone(), false));
                    out.classes
                        .push((name, class.to_interned_string(&interner)));
                }
                _ => {}
            },
            StatementListItem::Statement(statement) => {
                if let Statement::Var(var) = statement.as_ref() {
                    class_initializers(&var.0, &interner, &mut out.classes);
                }
            }
        }
    }
    out
}

/// String literals in `text` that may name states for `In()`. Only collected from text that
/// mentions `In`, since that is the only way such a string can reach the predicate.
pub(crate) fn in_candidates(text: &str) -> Vec<String> {
    if !text.contains("In") {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c != '\'' && c != '"' && c != '`' {
            continue;
        }
        let mut literal = String::new();
        let mut closed = false;
        while let Some(d) = chars.next() {
            if d == '\\' {
                if let Some(e) = chars.next() {
                    literal.push(e);
                }
            } else if d == c {
                closed = true;
                break;
            } else {
                literal.push(d);
            }
        }
        if closed
            && !literal.is_empty()
            && literal.len() <= 256
            && !literal.chars().any(char::is_whitespace)
        {
            out.push(literal);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expressions_cannot_escape_their_wrapper() {
        assert!(check_expression("x + 1").is_ok());
        assert!(check_expression("{a: 1}").is_ok());
        assert!(check_expression("function () {}").is_ok());
        assert!(check_expression("1 // trailing comment").is_ok());
        assert!(check_expression("1), (x = 5").is_err());
        assert!(check_expression("1); x = 5; (1").is_err());
        assert!(check_expression("1)} x = 5; {(1").is_err());
        assert!(check_expression("return").is_err());
        assert!(check_expression("").is_err());
    }

    #[test]
    fn locations_are_variables_or_members() {
        assert_eq!(parse_location("x"), Ok(Location::Variable("x".into())));
        assert_eq!(
            parse_location("a.b[0]"),
            Ok(Location::Member {
                root: Some("a".into())
            })
        );
        assert!(parse_location("f()").is_err());
        assert!(parse_location("a, b").is_err());
        assert!(parse_location("a) = 1; (b").is_err());
        assert!(parse_location("1").is_err());
        assert!(parse_location("a?.b").is_err());
        assert!(parse_variable("a.b").is_err());
    }

    #[test]
    fn top_level_bindings_are_found() {
        let found = script_bindings(
            "let a = 1, {b} = {b: 2}; const c = 3; class D {} var e = class {}; { let f; }",
        );
        assert_eq!(
            found.lexical,
            vec![
                ("a".to_string(), false),
                ("b".to_string(), false),
                ("c".to_string(), true),
                ("D".to_string(), false)
            ]
        );
        let names: Vec<&str> = found.classes.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, vec!["D", "e"]);
        assert!(found.classes[0].1.starts_with("class D"));
    }

    #[test]
    fn in_candidates_are_string_literals() {
        assert_eq!(in_candidates("In('s1') && In(\"s2\")"), vec!["s1", "s2"]);
        assert!(in_candidates("x == 'a'").is_empty());
    }
}
