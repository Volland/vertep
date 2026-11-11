//! The chart type language: one algebraic type language for event payloads, context and
//! executable content, shared by every surface ([[decisions#One IR type language]]).
//!
//! - [`Type`] is the language itself: the primitives `bool`, `int`, `decimal`, `string`,
//!   `datetime` and `iri`, records, tagged unions, lists, optional values, references to named
//!   types and the dynamic type `value`.
//! - [`syntax`] reads and prints the textual form used in `vt:type` attributes, the `vt:types`
//!   block and the YAML surface (`list<Order>`, `{ id: string }`, `Card { last4: string } | Cash`).
//! - [`check`] checks a runtime [`crate::datamodel::Value`] against a type, reporting every
//!   error with its path, and implements the canonical value encoding and the checked
//!   conversion from `value` to a static type.
//! - [`decl`] holds named type and event declarations and validates them (unknown names with
//!   suggestions, duplicates, cycles that are not guarded by a list or an optional).
//! - [`events`] answers which payload type an event or event descriptor has, including the
//!   predefined types of the SCXML system events.
//! - [`chart`] attaches types to IR nodes through [`crate::ir::Extension`] nodes (the IR has no
//!   typed fields) and type-checks a whole chart, refusing instantiation on error.
//!
//! Typing is opt-in: a chart without type annotations is untyped, every payload and context
//! value has type `value`, and the checker reports nothing ([[openspec:type-system#Opt-In Chart Typing]]).
//!
//! # Encoding of types in the IR
//!
//! | Annotation | IR carrier |
//! |---|---|
//! | `vt:type` on `<data>` | an `Extension` attribute `vt:type` on [`crate::ir::Data::extensions`] |
//! | `vt:type` on `<send>` | an `Extension` attribute `vt:type` on the send's [`crate::ir::Exec::extensions`] |
//! | `vt:type` on `<param>` | [`crate::ir::Param`] has no extensions, so the owner (send `Exec`, `Invoke` or final `State`) carries one attribute `vt:paramTypes` whose value is a record type `{ name: T, ... }` |
//! | `vt:types` block | an `Extension` element `vt:types` on [`crate::ir::Chart::extensions`] whose text is a declaration list (`type Order = {...}; event pay: Order;`) |

pub mod chart;
pub mod check;
pub mod decl;
pub mod events;
pub mod prim;
pub mod syntax;
mod ty;

#[cfg(test)]
mod tests;

pub use chart::{
    BasicTyper, ChartTypes, Diagnostic, DiagnosticKind, ExprError, ExprType, ExprTyper,
    InstantiateError, Scope, check_chart, compile_checked, is_typed,
};
pub use check::{PathSeg, TypeError, TypeErrorKind, ValuePath, check_value, convert, encode};
pub use decl::{DeclError, DeclErrorKind, Declarations, EventDecl, TypeDecl};
pub use events::EventTable;
pub use syntax::{SyntaxError, parse_declarations, parse_type};
pub use ty::{Field, Primitive, Type, Variant};
