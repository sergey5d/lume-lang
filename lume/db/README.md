# Lume DB

`lume/db` is the JDBC-backed database package for generated Java programs.
The query/exec/transaction API is written in Lume; handwritten Java is kept to
the small JDBC adapter layer that opens connections, binds SQL values, reads
rows, and converts `SQLException` into `DbError`.

The public API uses Lume core values at its boundary:

```lume
use lume/db/{Database, Row, DbError}

def loadUsers(db Database) Result[[User], DbError] {
    db.query("select id, name from users where active = ?")
        .bind(true)
        .decodeAll[User]()
}

def loadUser(db Database, id Int) Result[Option[User], DbError] {
    db.queryRow("select id, name from users where id = ?")
        .bind(id)
        .decodeOne[User]()
}

def updateUser(db Database, id Int, name Str) Result[Int, DbError] {
    db.exec("update users set name = ? where id = ?", name, id)
}

def updateUserStaged(db Database, id Int, name Str) Result[Int, DbError] {
    db.exec("update users set name = ? where id = ?")
        .bind(name, id)
        .run()
}

def replaceUser(db Database, id Int, name Str) Result[Unit, DbError] {
    db.transactionally(tx -> {
        try tx.exec("delete from users where id = ?", id)
        try tx.exec("insert into users(id, name) values (?, ?)", id, name)
        Ok(())
    })
}
```

Supported binding styles:

- Positional varargs: `db.query(sql).bind("Ada", 42)`
- Positional tuple: `db.query(sql).bind(("Ada", 42))`
- Positional list: `db.query(sql).bind(values)`
- Named SQL parameters through maps: `db.query(sql).bind(["name": "Ada"])`

Main execution forms:

- `db.query(sql).bind(...).rows()` returns all rows.
- `db.query(sql).bind(...).map(row -> pureValue(row))` maps all rows with a pure mapper.
- `db.query(sql).bind(...).flatMap(row -> decode(row))` decodes all rows with a mapper that returns `Result`.
- `db.query(sql).bind(...).decodeAll[User]()` decodes all rows into generated Lume class/shape values using reified type metadata.
- `db.queryRow(sql).bind(...).row()` returns `Option[Row]` and errors if more than one row is returned.
- `db.queryRow(sql).bind(...).map(row -> pureValue(row))` maps zero-or-one row with a pure mapper.
- `db.queryRow(sql).bind(...).flatMap(row -> decode(row))` decodes zero-or-one row with a mapper that returns `Result`.
- `db.queryRow(sql).bind(...).decodeOne[User]()` decodes zero-or-one row into a generated Lume class/shape value.
- `db.exec(sql, args...)` runs insert/update/delete/DDL immediately and returns affected row count.
- `db.exec(sql).bind(...).run()` is the staged/builder form for the same operation.

Reified row decoding:

- `decodeAll[T]()` and `decodeOne[T]()` use the hidden `Type[T]` evidence from `[reified T]`.
- `T` must currently be a generated Lume class or named shape with a public positional constructor.
- Row columns are matched to visible field names case-insensitively; SQL column order does not matter.
- Primitive fields are converted for `Str`, `Int`, `Float`, `Bool`, and `Rune`.
- Nullable columns map to `Option[...]` fields as `None`; non-null values become `Some(value)`.
- Use manual `map` / `flatMap` when decoding needs custom column names, joins, nested objects, interfaces, enums, or validation logic.

Named binding uses `:name` placeholders in SQL and lowers them to JDBC `?`
parameters. True anonymous-shape binding should be added when generated Java
can materialize anonymous shape literals at runtime. For now, shape-like named
binding is represented with `Map[Str, Any]`.
