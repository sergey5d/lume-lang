# Proposal: `Int` overflow panics by default

Status: implemented (runtime checking, explicit wrapping/checking APIs, and
backend parity tests). Optional compile-time constant-expression detection is
not implemented. The historical behaviour described below is as of commit
`f208f747`; current numeric semantics are authoritative in `syntax.md`.

## Summary

Today, `Int` arithmetic silently wraps modulo 2^64:

```txt
Int.MAX + 1                                  # → Int.MIN, with no diagnostic
[Int.MAX, 1].fold(0, (acc, x) => acc + x)    # → -9223372036854775808
(Int.MIN).abs()                              # → Int.MIN, a negative "absolute value"
2.pow(64)                                    # → 0
```

A wrong value keeps flowing through the program. That is the one place where
Lume's "nothing hidden" rule doesn't hold. Lume already panics on integer
division by zero, so a silently wrong result from overflow is inconsistent with
that.

**Proposal:** integer overflow panics, the same way division by zero does.
Explicit methods cover the cases where wrapping is intended (hashing, random
number generators, checksums), and where the caller wants to handle overflow
(`Option`).

## Current behaviour

### Spec

`syntax.md`, under "Numeric semantics":

- integer `+`, `-`, `*`, and unary `-` wrap modulo 2^64 on overflow
- the overflowing `Int.MIN / -1` case wraps to `Int.MIN`; `Int.MIN % -1` is zero

Also:

- `syntax.md:724`: "`Int.pow` … like other `Int` arithmetic, wraps on overflow".
- `syntax.md:3608`: warns that subtracting sort keys can wrap.

### Interpreter (`rust/crates/lume/src/interpreter.rs`)

| Operation | Line | Implementation |
|---|---|---|
| `+` | 6743 | `wrapping_add` |
| `-` | 6759 | `wrapping_sub` |
| `*` | 6768 | `wrapping_mul` |
| unary `-` | 6726 | `wrapping_neg` |
| `/` | 7714 | `wrapping_div` (zero is checked first, at 7713) |
| `%` | 7736 | `wrapping_rem` |
| `Int.abs()` | 5720 | `wrapping_abs` |
| `Int.pow()` | 5744, 5748 | `wrapping_mul` |

### Java backend

- `+`, `-`, `*`, `/` and `%` are emitted as plain Java operators
  (`java_backend/emit.rs:6828` and following, and `:2336` for compound
  assignment). They wrap.
- `LumeRuntime.abs` uses `Math.abs`, which wraps on `Long.MIN_VALUE`.
- `LumeRuntime.pow` uses plain `*=`.

### Already correct (no change needed)

- Integer literals outside the 64-bit range are compile errors.
- `Float.toInt()` clamps to `Int.MIN`/`Int.MAX` and turns NaN into `0`. This is
  a documented explicit conversion, not arithmetic.
- `Range(Int.MAX - 2, Int.MAX)` and `Range(Int.MIN + 1, Int.MIN, -1)` iterate
  correctly and terminate. Range iteration must keep this property; see the
  test list.

## Proposed semantics

### 1. Checked by default

These operations panic when the mathematical result does not fit in a signed
64-bit integer:

| Operation | Panics when |
|---|---|
| `a + b`, `a += b` | the result is out of range |
| `a - b`, `a -= b` | the result is out of range |
| `a * b`, `a *= b` | the result is out of range |
| `-a` | `a == Int.MIN` |
| `a / b`, `a /= b` | `a == Int.MIN && b == -1` (`b == 0` already panics) |
| `a.abs()` | `a == Int.MIN` |
| `a.pow(n)` | any intermediate product used for the result is out of range |

These are unchanged:

- `a % b` with `b == -1` stays `0`. The result is mathematically defined and
  representable, and it matches Rust's `checked_rem` and Java.
- `%` by zero keeps panicking.

Panic message format, matching the existing "integer division by zero" style:

```txt
integer overflow: 9223372036854775807 + 1
integer overflow: -(-9223372036854775808)
integer overflow: (-9223372036854775808).abs()
integer overflow: 3.pow(41)
```

The panic is an ordinary Lume panic: `LumePanic` on the JVM, and a runtime
error in the interpreter. Lume has no exceptions, so there is nothing to catch.
Code that needs to recover uses the checked methods below.

### 2. Explicit wrapping, for intentional modular arithmetic

New `Int` members:

```txt
def wrappingAdd(other Int) Int
def wrappingSub(other Int) Int
def wrappingMul(other Int) Int
def wrappingNeg() Int
```

Example:

```txt
def hash() Int = this.name.hash().wrappingMul(31).wrappingAdd(this.age)
```

The method name tells the reader that wrapping is intended. This is the same
design as `replaceAllRegex`: the special behaviour is in the name, never
implied.

### 3. Explicit checking, for recoverable overflow

```txt
def checkedAdd(other Int) Option[Int]
def checkedSub(other Int) Option[Int]
def checkedMul(other Int) Option[Int]
def checkedPow(exponent Int) Option[Int]
```

These return `None` on overflow, and also on a negative exponent for
`checkedPow`. They compose with the existing operators:

```txt
total = a.checkedAdd(b) ?? return Err("total too large")
```

No other variants are proposed; `saturating*` can be added later if needed.
Four `checked*` and four `wrapping*` methods cover the known use cases without
growing the API much.

### 4. Compile-time detection (optional, recommended)

When both operands are compile-time constants, report overflow as a compile
error and not a runtime panic. Lume already does this for out-of-range literals:

```txt
limit = Int.MAX + 1    # error[integer_overflow]: constant expression overflows Int
```

This is optional. The runtime rule alone is sound.

## Why panic, and not the alternatives

| Option | Verdict |
|---|---|
| **Wrap silently** (today; Java, Go) | Rejected. Wrong values propagate silently, which goes against "nothing hidden". |
| **Panic in debug builds, wrap in release** (Rust) | Rejected. The same program behaves differently depending on build mode, which is a second hidden behaviour. |
| **Promote to big integers** (Python) | Rejected. `Int` would silently change representation, contradicting the spec's "does not silently change representation", and it has a large performance cost. |
| **Always panic** (Swift, C# `checked`, Ada, Zig safe builds) | **Chosen.** It is consistent with division by zero, and there is one behaviour everywhere. |

Cost:

- In the Rust interpreter, `checked_add` and friends compile to an add followed
  by an overflow-flag branch. The cost is negligible.
- On the JVM, `Math.addExact`, `subtractExact`, `multiplyExact` and
  `negateExact` are JIT intrinsics. They compile to the same instruction plus a
  `jo` (jump-on-overflow) branch. HotSpot also removes the check when it can
  prove the range, which is typical in loop counters.

## Implementation

### Interpreter

- Replace `wrapping_add`, `wrapping_sub`, `wrapping_mul` and `wrapping_neg` at
  6726, 6743, 6759 and 6768 with `checked_*`, and on `None` produce
  `runtime_error(span, "integer overflow: …")`.
- `/` (7714): use `checked_div` after the existing zero check. `None` here means
  `Int.MIN / -1`.
- `%` (7736): keep `wrapping_rem`, so `Int.MIN % -1 == 0` stays.
- `abs` (5720): use `checked_abs`.
- `pow` (5744, 5748): use `checked_mul` for both `result` and `factor`. The
  existing loop already skips squaring `factor` after the last bit, so a
  `factor` overflow means the result would overflow too. Alternatively, use
  `i64::checked_pow`, which takes a `u32` exponent, after rejecting exponents
  that don't fit. Either way, `(-2).pow(63) == Int.MIN` must succeed.
- Add the eight new `wrapping*` and `checked*` members to the `Int` builtin
  dispatch.
- Compound assignments (`+=`, `-=`, `*=`, `/=`) must go through the same
  checked path. Verify they don't have a separate implementation.

### Java backend

- Emit `Math.addExact(a, b)`, `Math.subtractExact`, `Math.multiplyExact` and
  `Math.negateExact` for `Int` operands. `Float` operands keep plain operators.
- These throw `ArithmeticException`, not `LumePanic`. Either:
  - **(a)** emit calls to new `LumeRuntime.addInt`, `subInt`, `mulInt`, `negInt`
    and `divInt` helpers that call `Math.*Exact`, catch `ArithmeticException`
    and throw `LumePanic("integer overflow: …")` with the same message as the
    interpreter (recommended); or
  - **(b)** emit `Math.*Exact` directly and convert `ArithmeticException` to a
    Lume panic at the program boundary.

  Option (a) keeps messages identical across backends. The JIT inlines the
  helpers.
- `LumeRuntime.abs` (line 40): use `Math.absExact` (Java 15+), or check
  `Long.MIN_VALUE` explicitly.
- `LumeRuntime.pow` (line 48): use `Math.multiplyExact` for both `result` and
  `factor`, as in the interpreter.
- Compound assignments (`emit.rs:2336`): `x += y` must become
  `x = LumeRuntime.addInt(x, y)` and not the Java `+=` operator.

### Related backend difference to fix at the same time

Java integer `/` and `%` are emitted as raw operators. Division by zero
therefore throws `java.lang.ArithmeticException: / by zero`, with a JVM stack
trace. The interpreter instead reports `integer division by zero`. I found no
handler that converts the exception. Routing `/` and `%` through
`LumeRuntime.divInt`/`remInt` fixes this as part of the same change.

### Stdlib (`stdlib/numeric.lum`)

Add the declarations to `interface Int`:

```txt
def wrappingAdd(other Int) Int
def wrappingSub(other Int) Int
def wrappingMul(other Int) Int
def wrappingNeg() Int
def checkedAdd(other Int) Option[Int]
def checkedSub(other Int) Option[Int]
def checkedMul(other Int) Option[Int]
def checkedPow(exponent Int) Option[Int]
```

Audit existing code for intentional wrapping. Only `stdlib/reference_id.lum:12`
defines `hash()` in the repo today. Check whether it depends on wrapping, and
switch to `wrapping*` if it does.

### `syntax.md` changes

Replace these two bullets under "Numeric semantics":

```txt
- integer `+`, `-`, `*`, and unary `-` wrap modulo 2^64 on overflow
- the overflowing `Int.MIN / -1` case wraps to `Int.MIN`; `Int.MIN % -1` is zero
```

with:

```txt
- integer `+`, `-`, `*`, unary `-`, and their compound assignments panic when
  the result does not fit in a signed 64-bit integer
- `Int.MIN / -1` panics; `Int.MIN % -1` is zero
- `Int.abs()` panics for `Int.MIN`; `Int.pow` panics when the result overflows
- `wrappingAdd`, `wrappingSub`, `wrappingMul`, and `wrappingNeg` perform
  explicit two's-complement wrapping
- `checkedAdd`, `checkedSub`, `checkedMul`, and `checkedPow` return `None` on
  overflow
```

Also update:

- `syntax.md:724`: "Its multiplication, like other `Int` arithmetic, wraps on
  overflow" becomes "It panics when the result overflows; use `checkedPow` to
  handle overflow as `None`".
- `syntax.md:3608`: "subtraction can wrap" becomes "subtraction can overflow
  and panic".

## Tests

Add to `examples/` with headers that both backends run:

- **`examples/features/int_overflow_explicit.lum`:**
  - `wrapping*` results at the boundaries (`Int.MAX.wrappingAdd(1) == Int.MIN`
    and so on).
  - `checked*` returning `Some` and `None`.
  - `checkedPow` with a negative exponent returning `None`.
  - `Int.MIN % -1 == 0`.
  - `Range(Int.MAX - 2, Int.MAX)` and `Range(Int.MIN + 1, Int.MIN, -1)` still
    terminating. Range stepping must not panic at the last element.
- **`examples/failures/`**, one file per panic, with `EXPECT-ERROR` on the
  message:
  - `int_overflow_add.lum`: `Int.MAX + 1`
  - `int_overflow_sub.lum`: `Int.MIN - 1`
  - `int_overflow_mul.lum`: `Int.MAX * 2`
  - `int_overflow_neg.lum`: `-Int.MIN` through a variable, so constant folding
    does not apply
  - `int_overflow_div.lum`: `Int.MIN / -1`
  - `int_overflow_abs.lum`: `Int.MIN.abs()`
  - `int_overflow_pow.lum`: `3.pow(41)`
  - `int_overflow_compound.lum`: `x += 1` where `x == Int.MAX`
  - `int_overflow_fold.lum`: `[Int.MAX, 1].fold(0, (acc, x) => acc + x)`
- **Java parity:** the same failure files must produce the same panic message
  on the Java backend, including the existing `integer division by zero` case.
- **Edge cases for `pow`:**
  - `3.pow(39)` (about 4.05e18) succeeds and `3.pow(40)` (about 1.2e19)
    panics.
  - `2.pow(62)` succeeds and `2.pow(63)` panics.
  - `(-2).pow(63) == Int.MIN` succeeds. Its magnitude is 2^63, but the negative
    result fits.
