//! Compile-time scenarios of the rust-dsl spec, as trybuild pass and compile-fail cases. The
//! expected compiler output is in `tests/ui/*.stderr`.

fn cases() -> trybuild::TestCases {
    trybuild::TestCases::new()
}

// @tg: verifies:: [[openspec:rust-dsl#Statechart Macro#Syntax error is located]] {level: integration}
#[test]
fn syntax_error_is_located() {
    cases().compile_fail("tests/ui/syntax_error.rs");
}

// @tg: verifies:: [[openspec:rust-dsl#Generated Typed API#Wrong payload does not compile]] {level: integration}
#[test]
fn wrong_payload_does_not_compile() {
    cases().compile_fail("tests/ui/wrong_payload.rs");
}

// @tg: verifies:: [[openspec:rust-dsl#Compile-Time Structural Checks#Unknown target]] {level: integration}
#[test]
fn unknown_target() {
    cases().compile_fail("tests/ui/unknown_target.rs");
}

// @tg: verifies:: [[openspec:rust-dsl#Compile-Time Structural Checks#Duplicate state id]] {level: integration}
#[test]
fn duplicate_state_id() {
    cases().compile_fail("tests/ui/duplicate_state.rs");
}

// @tg: verifies:: [[openspec:rust-dsl#Compile-Time Structural Checks#Several violations in one chart]] {level: integration}
#[test]
fn several_violations_in_one_chart() {
    cases().compile_fail("tests/ui/several_violations.rs");
}

// @tg: verifies:: [[openspec:rust-dsl#Compile-Time Reachability and Coverage Diagnostics#Unreachable state warning]] {level: integration}
#[test]
fn unreachable_state_warning() {
    let t = cases();
    // Compiles and runs: the finding is a warning ...
    t.pass("tests/ui/unreachable_state_warns.rs");
    // ... a `deprecated` lint naming `archived`, shown here by denying it.
    t.compile_fail("tests/ui/unreachable_state.rs");
}

// @tg: verifies:: [[openspec:rust-dsl#Compile-Time Reachability and Coverage Diagnostics#Unhandled event as error]] {level: integration}
#[test]
fn unhandled_event_as_error() {
    cases().compile_fail("tests/ui/unhandled_event.rs");
}

// @tg: verifies:: [[openspec:rust-dsl#Guard and Action Signature Checking#Guard with wrong event type]] {level: integration}
#[test]
fn guard_with_wrong_event_type() {
    cases().compile_fail("tests/ui/guard_wrong_event.rs");
}

// @tg: verifies:: [[openspec:rust-dsl#Guard and Action Signature Checking#Missing function]] {level: integration}
#[test]
fn missing_function() {
    cases().compile_fail("tests/ui/missing_function.rs");
}

// @tg: verifies:: [[openspec:rust-dsl#Guard and Action Signature Checking#Guard tries to mutate context]] {level: integration}
#[test]
fn guard_tries_to_mutate_context() {
    cases().compile_fail("tests/ui/guard_mut_ctx.rs");
}

// @tg: verifies:: [[openspec:rust-dsl#Pure Guards#Guard cannot emit events]] {level: integration}
#[test]
fn guard_cannot_emit_events() {
    let t = cases();
    // A guard that asks for an emitter does not fit the guard signature ...
    t.compile_fail("tests/ui/guard_emit.rs");
    // ... and it cannot make one: only the datamodel constructs an `Emit`.
    t.compile_fail("tests/ui/guard_builds_emitter.rs");
}
