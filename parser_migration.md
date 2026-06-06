# Parser Migration Plan

## Goal

Split the former `rust/crates/lume/src/parser.rs` into the current [rust/crates/lume/src/parser/](/Users/sergeyd/Projects/a-lang/rust/crates/lume/src/parser) module tree without changing parser behavior or the public parsing entry points.

Status: complete.

The current file is a good fit for a mechanical split because it already has strong internal regions:

- program and item parsing
- statement parsing
- pattern and refutable-binding parsing
- expression parsing
- type parsing
- parser support and recovery helpers
- string/interpolation helpers
- tests

## Core Design Decision

Keep a single `Parser<'a>` state machine.

Do **not** introduce a trait/interface-per-node parsing architecture. The parser is driven by one shared mutable cursor plus shared diagnostics, so the natural owner of the parsing logic is still `impl Parser`, just spread across modules.

That means the end state should look like:

```txt
src/parser/
  mod.rs
  items.rs
  stmt.rs
  pattern.rs
  expr.rs
  types.rs
  support.rs
  strings.rs
  tests.rs
```

Each file continues to contain `impl<'a> Parser<'a>` blocks for its own method cluster.

## Non-Goals

- no grammar changes
- no AST changes
- no lexer changes
- no behavior changes in diagnostics or recovery unless needed to preserve compilation after the split
- no trait-based parser rewrite

## Public API To Preserve

These should remain stable:

- `parse_program(tokens: &[Token]) -> ParseResult`
- `ParseResult`
- parser tests and example parity behavior

## Target Structure

### `parser/mod.rs`

Keep only the entry surface and shared parser state:

- `ParseResult`
- `parse_program(...)`
- `Parser<'a>`
- `Checkpoint`
- `Parser::new`
- `Parser::parse_program`

Also declare the internal modules:

```rust
mod items;
mod stmt;
mod pattern;
mod expr;
mod types;
mod support;
mod strings;

#[cfg(test)]
mod tests;
```

### `parser/items.rs`

Top-level declarations and imports/modules.

Move:

- `parse_module_decl`
- `parse_import_decl`
- `parse_item`
- `parse_visibility`
- `parse_annotations`
- `parse_import_segments`
- `parse_import_symbol_list`
- `parse_function_decl`
- `parse_type_decl`
- `parse_enum_case`
- `parse_impl_block`
- `parse_method_decl`
- `parse_field_decl`
- `parse_callable_body`

### `parser/stmt.rs`

Statements and blocks.

Move:

- `parse_block`
- `parse_stmt`
- `parse_binding_stmt_after_var`
- `parse_let_stmt`
- `try_parse_binding_stmt`
- `parse_binding`
- `parse_binding_list`
- `is_binding_start`
- `try_parse_assignment_stmt`
- `parse_if_stmt`
- `parse_while_stmt`
- `parse_for_stmt`
- `parse_unwrap_stmt`
- `parse_unwrap_block_stmt`
- `parse_match_stmt`
- `parse_match_cases`
- `parse_return_stmt`
- `parse_break_stmt`

### `parser/pattern.rs`

Patterns and refutable clauses.

Move:

- `parse_pattern`
- `parse_pattern_at_depth`
- `parse_refutable_clause`
- `parse_if_condition_refutable_clause`
- `parse_refutable_clause_block`
- `parse_if_condition_clauses`
- `parse_if_condition_expr`

### `parser/expr.rs`

General expression parsing and precedence climbing.

Move:

- `parse_expr`
- `parse_expr_without_trailing_block_call`
- `try_parse_lambda_expr`
- `parse_lambda_param`
- `parse_lambda_body`
- `parse_if_expr`
- `parse_match_expr_after_keyword`
- `parse_for_yield_expr_after_start`
- `parse_for_binding_block`
- `parse_yield_body_block`
- `parse_record_literal_expr`
- `parse_brace_record_literal_expr`
- `finish_brace_record_literal_expr`
- `is_anonymous_interface_expr_start`
- `parse_anonymous_interface_expr`
- `parse_record_update_args`
- `parse_then_stmt_body_block`
- `parse_block_or_inline_stmt_body`
- `parse_then_expr_body_block`
- `parse_block_or_inline_expr_body`
- `parse_colon_expr`
- `parse_or_expr`
- `parse_bit_or_expr`
- `parse_and_expr`
- `parse_bit_and_expr`
- `parse_equality_expr`
- `parse_comparison_expr`
- `parse_term_expr`
- `parse_factor_expr`
- `parse_left_assoc`
- `parse_unary_expr`
- `parse_postfix_expr`
- `parse_call_args`
- `parse_primary_expr`
- `is_bare_record_call_arg_start`
- `parse_list_literal`
- `parse_group_or_tuple_expr`
- `parse_expr_list`

### `parser/types.rs`

Type parsing and generic parameters.

Move:

- `parse_type_params`
- `parse_param_list`
- `parse_optional_return_type`
- `parse_type_ref_list`
- `parse_type_ref`
- `parse_primary_type_ref`
- `parse_tuple_type_field`
- `can_start_type_ref`
- `parse_path_string`

### `parser/support.rs`

Parser runtime utilities, token movement, lookahead, diagnostics, and recovery.

Move:

- `synchronize_item`
- `synchronize_member`
- `synchronize_stmt`
- `checkpoint`
- `restore`
- `skip_newlines`
- `consume`
- `consume_keyword`
- `expect_identifier`
- `expect_binding_name`
- `parse_callable_name`
- `match_keyword`
- `at_keyword`
- `match_token`
- `at`
- `at_next`
- `binding_type_starts_on_same_line`
- `pattern_followed_by_eq`
- `scan_if_condition_expr_end`
- `is_placeholder_identifier`
- `is_for_yield_start`
- `current`
- `current_kind`
- `current_span`
- `previous_span`
- `last_non_newline_span`
- `next_significant_token`
- `next_significant_token_string`
- `current_token_string`
- `format_token_like`
- `token_kind_label`
- `advance`
- `error_at_current`

### `parser/strings.rs`

String literal and interpolation helpers.

Move:

- `parse_string_expr`
- `is_multiline_string`
- `string_has_interpolation`
- `decode_string_contents`
- `encode_string_literal`
- `parse_interpolated_string_parts`
- `find_interpolated_expr_end`
- `parse_embedded_expr`

Note:

- `starts_lower` ended up living in `items.rs`, because it is only used by import-segment parsing.

### `parser/tests.rs`

Move the entire `#[cfg(test)]` block out of `mod.rs`.

This should include:

- parse helpers used only in tests
- repo source sweep tests
- parser unit tests

## Recommended Migration Order

Do not move everything at once.

### Phase 1: Extract Low-Risk, Self-Contained Code

Create the directory/module structure and move:

- `tests.rs`
- `strings.rs`

Why first:

- they have low coupling to parser control flow
- they reduce file size immediately
- they are easy to validate with parser tests

### Phase 2: Extract `support.rs`

Move parser mechanics and recovery helpers next.

Why second:

- all other modules depend on these helpers
- once this is stable, later file moves become mostly mechanical

### Phase 3: Extract `types.rs`

Move type parsing next.

Why:

- type parsing is already a well-bounded region
- it has fewer cyclic dependencies than expressions/statements

### Phase 4: Extract `pattern.rs`

Move pattern/refutable-clause parsing.

Why:

- pattern parsing is now a real subsystem of its own
- it reduces complexity in both `stmt` and `expr`

### Phase 5: Extract `items.rs`

Move module/import/top-level declaration parsing.

Why:

- declaration parsing is cohesive
- this makes `mod.rs` much smaller and more legible

### Phase 6: Extract `stmt.rs`

Move block and statement parsing.

Why:

- statement parsing is large but still manageable once support/pattern/items are already out

### Phase 7: Extract `expr.rs`

Move expression parsing last.

Why:

- this is the highest-coupling area
- it depends on body helpers, lambdas, call parsing, partial/match shorthand, and postfix behavior

## Implementation Notes

### 1. Use multiple `impl Parser` blocks

Each file should keep methods inside `impl<'a> Parser<'a>`.

That gives:

- no change to ownership model
- no trait indirection
- minimal call-site churn

### 2. Keep helper visibility private

Keep parser internals scoped to the `parser` module tree.

In practice, sibling module extraction requires `pub(super)` on moved helper methods so they remain callable across `items.rs` / `stmt.rs` / `expr.rs` / `types.rs` / `pattern.rs`, without exposing them outside `parser`.

### 3. Prefer moving complete clusters

Do not split tightly related method families across files just to equalize file sizes.

Example:

- all precedence methods should stay together in `expr.rs`
- all token/recovery helpers should stay together in `support.rs`

### 4. Avoid early micro-splitting

Do not start with:

- `expr_primary.rs`
- `expr_postfix.rs`
- `stmt_binding.rs`
- `stmt_control.rs`

Those may be useful later, but the first split should aim for obvious subsystem boundaries, not maximal fragmentation.

## Known Risk Areas

### `parse_if_stmt`

Touches:

- statements
- refutable clauses
- pattern helpers
- inline body parsing

Mitigation:

- move `pattern.rs` before `stmt.rs`

### `try_parse_lambda_expr`

Touches:

- expressions
- params
- block/expression body handling

Mitigation:

- keep all lambda-related methods in `expr.rs`

### `parse_postfix_expr`

Touches:

- call args
- record updates
- match/partial shorthand
- trailing block behavior

Mitigation:

- move the full postfix family together, not piecemeal

### Test helpers

Tests at the bottom of the current file use local parse helpers and repo sweeps.

Mitigation:

- move the test helper functions with the tests into `tests.rs`
- keep only production parser code in the non-test modules

## Acceptance Criteria

The migration is done when:

- [x] `parser.rs` becomes `parser/mod.rs`
- [x] the parser compiles as a directory module
- [x] all current parser tests pass
- [x] Rust crate tests pass:
  - `cargo test --manifest-path rust/Cargo.toml -p lume`
- [x] example parity still passes:
  - `./run_samples.sh rust`
- [x] there are no behavior or diagnostic regressions beyond intentional file/module moves

## Suggested First PR

The safest first PR is intentionally small:

1. create `src/parser/`
2. move current `parser.rs` to `parser/mod.rs`
3. extract `strings.rs`
4. extract `tests.rs`
5. extract `support.rs`
6. run formatter and Rust tests

This gives immediate structure without touching the most fragile grammar code first.

## Suggested Second PR

1. extract `types.rs`
2. extract `pattern.rs`
3. run Rust tests and example parity

## Suggested Third PR

1. extract `items.rs`
2. extract `stmt.rs`
3. extract `expr.rs`
4. do final cleanup of imports and module ordering

Result:

- completed

## Optional Follow-Up Cleanup

After the split lands and stabilizes:

- consider renaming a few functions for consistency if needed
- consider a smaller `stmt_bindings.rs` / `stmt_control.rs` split only if `stmt.rs` remains too large
- consider a smaller `expr_atoms.rs` / `expr_postfix.rs` split only if `expr.rs` remains difficult to navigate

Those should be follow-ups, not part of the first migration.
