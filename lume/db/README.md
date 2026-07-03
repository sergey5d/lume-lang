# Lume DB

`lume/db` is the JDBC-backed database package for generated Java programs.

The public API uses Lume core values at its boundary:

```lume
use lume/db/{Database, Row, DbError}

def loadUsers(db Database) Result[[User], DbError] {
    db.query("select id, name from users where active = ?")
        .bind(true)
        .map(row -> readUser(row))
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
- Named SQL parameters through maps: `db.query(sql).bind(Map("name": "Ada"))`

Main execution forms:

- `db.query(sql).bind(...).rows()` returns all rows.
- `db.query(sql).bind(...).map(row -> decode(row))` decodes all rows.
- `db.queryRow(sql).bind(...).row()` returns `Option[Row]` and errors if more than one row is returned.
- `db.queryRow(sql).bind(...).map(row -> decode(row))` decodes zero-or-one row.
- `db.exec(sql, args...)` runs insert/update/delete/DDL immediately and returns affected row count.
- `db.exec(sql).bind(...).run()` is the staged/builder form for the same operation.

Named binding uses `:name` placeholders in SQL and lowers them to JDBC `?`
parameters. True anonymous-shape binding should be added when generated Java
can materialize anonymous shape literals at runtime. For now, shape-like named
binding is represented with `Map[Str, Any]`.
