# Lume DB

`lume/db` is the JDBC-backed database package for generated Java programs.

The public API uses Lume core values at its boundary:

```lume
use lume/db/{Database, Row, DbError}

def loadUsers(db Database) Result[[User], DbError] {
    db.query("select id, name from users where active = ?")
        .bind(true)
        .map(row -> lift {
            id: row.int("id")
            name: row.str("name")
        }.map(shape -> User { ...shape }))
}
```

Supported binding styles:

- Positional varargs: `query.bind("Ada", 42)`
- Positional list: `query.bindAll(values)`
- Positional tuple: `query.bindTuple(("Ada", 42))`
- Named SQL parameters through maps: `query.bindNamed(Map("name": "Ada"))`

Named binding uses `:name` placeholders in SQL and lowers them to JDBC `?`
parameters. True anonymous-shape binding should be added when generated Java
can materialize anonymous shape literals at runtime.
