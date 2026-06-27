# Singleton Surface Notes

This note used to track whether user-defined `object` declarations should be
removed. The current direction is settled enough to document differently:

- `single` is the public singleton declaration syntax
- `impl single Name { ... }` is the behavior block for singleton methods
- `object` may still appear in older parser/runtime internals and legacy tests,
  but new language docs and examples should use `single`

## Current Shape

```txt
single OSLike {
}

impl single OSLike {
    def println(value Str) Unit = OS.println(value)
}
```

Singletons are useful for:

- namespaced utility methods
- runtime-provided values such as `OS`
- companion-like factory helpers when namespacing is useful
- long-lived shared state when the language intentionally allows it

## Why `single` Instead Of `object`

`object` was doing too much conceptual work:

- namespace
- singleton value
- companion/factory holder
- possible state container

`single` makes the singleton nature explicit while avoiding the confusion that a
class instance and a singleton declaration are both "objects".

## Companion / Factory Question

The remaining design question is whether same-named singles should ever get
privileged factory access to class internals.

Example shape:

```txt
class User {
    name Str
    hidden token Str
}

single User {
}

impl single User {
    def fromToken(name Str, token Str) User = ...
}
```

Open questions:

- can `single User` access hidden fields of class `User`?
- does same-name privilege make construction clearer, or does it add hidden magic?
- should this stay an ordinary factory namespace with no special hidden-field access?
- should explicit `new { ... } { ... }` remain the only way to initialize hidden fields?

## Current Leaning

Keep `single` as an ordinary singleton/namespace mechanism for now. Treat any
hidden-field companion privilege as a separate feature, not as something
implied by sharing a name.
