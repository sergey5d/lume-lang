# Lume JSON

`lume/json` provides encode-only JSON support for now.

```lume
use lume/json/{Json, JsonName}

class User {
    @JsonName { value: "user_name" }
    name Str

    age Int

    hidden token Str = "secret"
}

text Str = Json.stringify(User { name: "Ada", age: 42 })
```

Hidden fields are not serialized. `@JsonName` can rename a visible field and
`@JsonIgnore` can omit one explicitly. The language-facing entry points are
declared in Lume: `annotation JsonName`, `annotation JsonIgnore`, `JsonField`,
`JsonValue`, and `single Json`. Low-level escaping, collection traversal, and
reflection caching live in the small JVM bridge used by that Lume facade.
