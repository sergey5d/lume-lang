# Keyed Construction

Keyed construction lets map-like types accept brace entries whose keys are
expressions rather than construction field labels.

## Shape

```txt
impl single CustomHashMap {
    keyed[K, V](entries [(K, V)]) CustomHashMap[K, V] {
        map = CustomHashMap[K, V] {}

        for let (key, value) <- entries {
            map.put(key, value)
        }

        map
    }
}
```

Then this:

```txt
m = CustomHashMap {
    "a": 1
    "b": 2
}
```

lowers to:

```txt
m = CustomHashMap.keyed([
    ("a", 1),
    ("b", 2)
])
```

Conceptually:

```txt
keyed constructor = constructor from key/value entries
```

## Why `keyed`

```txt
keyed[K, V](entries [(K, V)]) CustomHashMap[K, V]
```

This tells the language directly:

```txt
This type accepts keyed construction syntax.
```

## Rule

For:

```txt
T { key: value }
```

Resolution:

1. Identifier entries such as `name: value` are construction fields.
2. Expression-key entries such as `"name": value`, `42: value`, or `(key): value` are keyed entries.
3. The compiler collects keyed entries into `[(K, V)]`.
4. The compiler calls `T.keyed(entries)`.
5. If `T` has no matching `keyed` method, report an error.

Example:

```txt
m CustomHashMap[Str, Int] = CustomHashMap {
    "a": 1
    "b": 2
}
```

Compiler checks:

```txt
"a" -> Str
1   -> Int

entries [(Str, Int)]
```

Then calls roughly:

```txt
CustomHashMap.keyed[Str, Int]([
    ("a", 1),
    ("b", 2)
])
```

## Class Or Companion

For map-like generic types, `keyed` should live on the companion/single:

```txt
impl single CustomHashMap {
    keyed[K, V](entries [(K, V)]) CustomHashMap[K, V] {
        ...
    }
}
```

It should not live inside the instance class body because keyed construction
creates the instance.

For built-in maps:

```txt
impl single Map {
    keyed[K, V](entries [(K, V)]) Map[K, V] {
        ...
    }
}
```

For custom maps:

```txt
impl single CustomHashMap {
    keyed[K, V](entries [(K, V)]) CustomHashMap[K, V] {
        ...
    }
}
```

## Keeping `:` Clean

With keyed construction, `:` has one syntactic role inside construction braces:

```txt
keyed element inside `{ ... }`
```

The meaning of the key depends on the target type:

```txt
User {
    name: "Ada"     # field label
}

Map {
    "name": "Ada"   # key expression
}
```

There is no global pair syntax:

```txt
x = "a": 1          # invalid
```

Pair values stay ordinary tuples:

```txt
x = ("a", 1)
```

## Summary

```txt
impl single Map {
    keyed[K, V](entries [(K, V)]) Map[K, V] {
        ...
    }
}
```

`keyed` as a constructor kind accepting a list of tuples is the settled design:

- cleaner than magic `fromEntries`
- cleaner than overloading `:`
- more extensible than hardcoding `Map`
