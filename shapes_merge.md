# Shape Composition and Update

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

Composition is collision-safe. An accidental unresolved overlap is an error.

```lume
# Error when both values contain `name`.
invalid = {
    ...left
    ...right
}
```

The diagnostic identifies both providers and requires an explicit field choice
or `override` intent.

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

An explicit `field: value` resolves that field regardless of whether it appears
before or after the colliding spreads. It may also add a field that was not
present in any spread. Duplicate explicit fields remain invalid.

`override ...b` makes collisions from that spread intentional:

- fields unique to `b` are added
- fields shared with earlier entries are replaced by values from `b`
- field types must remain valid for the resulting statically known shape
- a later ordinary spread can make a field ambiguous again

An ordinary `...b` remains collision-protected. The programmer must write
`override ...b` when replacement is intended.

The compiler analyzes the complete literal rather than rejecting a collision
as soon as it is encountered:

```lume
point = { x: 1, y: 2 }
dot = { x: 3, time: 4 }

selected = {
    ...point
    ...dot
    x: point.x
}
```

Here `x` has an explicit winner, while `y` and `time` each have one provider.
If both source shapes later gain another shared field, that new collision is a
compile-time error until it is explicitly resolved. By contrast,
`override ...dot` deliberately accepts all current and future overlaps from
`dot`, which is useful for configuration layering.

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
