# Lume JSON

`lume/json` provides typed JSON encoding and decoding.

```lume
use lume/json/{Json, JsonName}

class User {
    @JsonName { value: "user_name" }
    name Str

    age Int

    private token Str = "secret"
}

text Str = Json.stringify(User { name: "Ada", age: 42 })

decoded Result[User, Str] = Json.decode[User](
    """{"user_name":"Bob","age":31}"""
)
```

Private fields are not serialized. `@JsonName` can rename a visible field and
`@JsonIgnore` can omit one explicitly. The language-facing entry points are
declared in Lume: `annotation JsonName`, `annotation JsonIgnore`, `JsonField`,
`JsonValue`, and `object Json`.

The Rust interpreter provides this module natively, so `lume run` does not need
the JSON JVM artifact. Generated Java uses the corresponding JVM runtime bridge.
