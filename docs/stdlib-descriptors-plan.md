# Descriptor-Driven Standard Library

## Contract

Each interpreter standard-library type is declared in Rust under
`rust/crates/lume/src/stdlib`. Its descriptor supplies the complete public
contract, native implementation bindings, and supported intrinsic operations.
Normal user declarations and Java stubs are normalized to the same declaration
model. The parser has no standard-library member signatures.

Compilation resolves members and checks names, overloads, generics, defaults,
visibility, and argument types through these contracts. Interpretation invokes
the selected implementation binding instead of switching on type/member names.
Intrinsic arithmetic, comparison, extraction, indexing, and iteration have
explicit capability entries. Native representations are closed, trusted core
bindings, not types that user declarations can impersonate.

## Implementation Plan

1. Introduce full Rust-authored library contracts and explicit operator metadata.
   Preserve named parameters, bounds, reified parameters, defaults, varargs,
   getters, constructors, companions, aliases, and union alternatives.
2. Move native implementations from `runtime/builtins` to `stdlib/native`.
   Derive runtime signatures from the same contracts and assign dense method
   slots. Keep per-program IR layouts separate from reusable library metadata.
3. Load ambient contracts through the registry. Use normal call checking for
   factories, printing, panic, and collection constructors; infer generic
   arguments through declared supertypes rather than special factory handlers.
4. Replace interpreter constructor, static, getter, and instance dispatch with
   descriptor bindings. Preserve callable boundaries, evaluation order, shared
   collection/handle identity, error behavior, and initialization order.
5. Preserve the Java stub adapter and normalize its declarations through the
   shared descriptor boundary. Replace duplicated lowering signature tables
   with registry lookups where applicable.
6. Add descriptor completeness, full-signature, operator availability, generic
   factory, invalid-call, no-filesystem-contract, and backend regression tests.

## Verification Baseline

Before this refactor, the library suite has 885 passing tests and three failing
corpus checks. All three fail at the existing invalid `yield` statement in
`examples/features/for.lum:15`. That example is not part of this change.

## Scope Boundaries

Keep existing language semantics and API names. This work does not redesign
numeric conversions, JSON encoding policies, Java generics, reference identity,
or visibility. Native implementation internals may still match on values or
operation kinds; the removed special cases are member/signature dispatch.
