# Shape Composition and Update

> Design proposal. The current language reference still documents `:<` for
> exact-shape updates. This proposal replaces that surface with `with`.

Lume needs three visible operations, but they reduce to two underlying
mechanisms:

1. shape construction and spread
2. exact-shape update with `with`

## Composition

Ordinary spreads compose visible fields into a fresh anonymous shape:

```lume
composed = {
    ...a
    ...b
}
```

Composition is collision-safe. The source shapes must have distinct field
names; an accidental overlap is an error.

```lume
# Error when both values contain `name`.
invalid = {
    ...left
    ...right
}
```

The diagnostic should identify the duplicate field and require explicit
override intent.

## Construction Overrides

Shape construction can deliberately replace fields while producing a fresh
shape:

```lume
replaced = {
    ...a
    field: replacement
    override ...b
}
```

An explicit `field: value` after a spread deliberately replaces that field.
It may also add a field that was not present in an earlier spread.

`override ...b` makes collisions from that spread intentional:

- fields unique to `b` are added
- fields shared with earlier entries are replaced by values from `b`
- field types must remain valid for the resulting statically known shape

An ordinary `...b` remains collision-protected. The programmer must write
`override ...b` when replacement is intended.

## Exact-Shape Update

Use `with` to update an existing value without changing its shape:

```lume
updated = a with {
    field: replacement
}
```

Rules:

- every field in the update must already exist on `a`
- an update cannot add fields
- replacement values must be assignable to the existing field types
- hidden fields are not updateable through this surface
- the result preserves the source's static shape
- the source value is not mutated; the expression produces an updated value

## Mental Model

```text
shape construction/spread
    creates a fresh shape
    can add fields
    collisions require explicit intent

with
    updates only existing fields
    cannot add fields
    preserves the source shape
```

The three surface forms therefore have one clear job each:

```lume
# Protected composition.
composed = {
    ...a
    ...b
}

# Construction with deliberate replacement.
replaced = {
    ...a
    field: replacement
    override ...b
}

# Exact-shape update.
updated = a with {
    field: replacement
}
```
