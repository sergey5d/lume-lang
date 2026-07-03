package lume.db;

import java.util.ArrayList;
import java.util.function.Function;

import lume.core.LumeList;
import lume.core.Result;

public final class Query {
    private final QueryRunner runner;
    private final String sql;
    private final SqlBindings bindings;

    Query(QueryRunner runner, String sql) {
        this(runner, sql, SqlBindings.empty());
    }

    private Query(QueryRunner runner, String sql, SqlBindings bindings) {
        this.runner = runner;
        this.sql = sql;
        this.bindings = bindings;
    }

    public Query bind(Object... values) {
        return new Query(runner, sql, SqlBindings.from(values));
    }

    public Result<LumeList<Row>, DbError> rows() {
        return runner.run(connection ->
            Jdbc.query(connection, sql, bindings.positional(), bindings.named())
        );
    }

    public Result<LumeList<Row>, DbError> all() {
        return rows();
    }

    public <T> Result<LumeList<T>, DbError> map(Function<Row, Result<T, DbError>> mapper) {
        var rows = rows();
        if (rows instanceof Result.Err<?, ?> err) {
            @SuppressWarnings("unchecked")
            var error = (DbError) err.error();
            return new Result.Err<>(error);
        }

        @SuppressWarnings("unchecked")
        var ok = (Result.Ok<LumeList<Row>, DbError>) rows;
        var out = new ArrayList<T>();
        for (var row : ok.value().asJava()) {
            var mapped = mapper.apply(row);
            if (mapped instanceof Result.Err<?, ?> err) {
                @SuppressWarnings("unchecked")
                var error = (DbError) err.error();
                return new Result.Err<>(error);
            }
            @SuppressWarnings("unchecked")
            var mappedOk = (Result.Ok<T, DbError>) mapped;
            out.add(mappedOk.value());
        }
        return new Result.Ok<>(LumeList.from(out));
    }

    public <T> Result<LumeList<T>, DbError> mapValue(Function<Row, T> mapper) {
        var rows = rows();
        if (rows instanceof Result.Err<?, ?> err) {
            @SuppressWarnings("unchecked")
            var error = (DbError) err.error();
            return new Result.Err<>(error);
        }

        @SuppressWarnings("unchecked")
        var ok = (Result.Ok<LumeList<Row>, DbError>) rows;
        var out = new ArrayList<T>();
        for (var row : ok.value().asJava()) {
            out.add(mapper.apply(row));
        }
        return new Result.Ok<>(LumeList.from(out));
    }
}
