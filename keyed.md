# Keyed Construction

This note sketches a possible keyed construction feature for map-like types.

## Shape

```txt
impl single CustomHashMap {
    keyed[K, V](entries [(K, V)]) CustomHashMap[K, V] {
        map = CustomHashMap[K, V] {}

        for (key, value) <- entries {
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

## Why `keyed` Instead Of Magic `fromEntries`

Using a magic method name is workable:

```txt
def fromEntries(...)
```

But it is less language-like because the compiler has to know that this method
name is special.

A dedicated constructor kind is clearer:

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

Resolution should be:

1. If `T` has normal field or constructor-shape construction, treat keys as
   field labels.
2. Else if `T` has a `keyed` constructor, treat keys as expressions and values
   as expressions.
3. Collect keyed entries into `[(K, V)]`.
4. Call the keyed constructor.
5. Otherwise, report an error.

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

Then calls:

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

## Preferred Syntax

```txt
impl single Map {
    keyed[K, V](entries [(K, V)]) Map[K, V] {
        ...
    }
}
```

So `keyed` as a constructor kind accepting a list of tuples is a solid design:

- cleaner than magic `fromEntries`
- cleaner than overloading `:`
- more extensible than hardcoding `Map`
