package lume.db;

import java.util.function.Function;

import lume.core.LumeRuntime;
import lume.core.Option;
import lume.core.Result;

public final class RowQuery {
    private final Query query;

    RowQuery(QueryRunner runner, String sql) {
        this(new Query(runner, sql));
    }

    private RowQuery(Query query) {
        this.query = query;
    }

    public RowQuery bind(Object... values) {
        return new RowQuery(query.bind(values));
    }

    public Result<Option<Row>, DbError> row() {
        var rows = query.rows();
        if (rows instanceof Result.Err<?, ?> err) {
            @SuppressWarnings("unchecked")
            var error = (DbError) err.error();
            return new Result.Err<>(error);
        }

        @SuppressWarnings("unchecked")
        var ok = (Result.Ok<lume.core.LumeList<Row>, DbError>) rows;
        var count = ok.value().size();
        if (count == 0L) {
            return Jdbc.ok(LumeRuntime.optionNone());
        }
        if (count > 1L) {
            return Jdbc.err("queryRow expected at most one row, got " + count);
        }
        return Jdbc.ok(ok.value().get(0));
    }

    public <T> Result<Option<T>, DbError> map(Function<Row, Result<T, DbError>> mapper) {
        var maybeRow = row();
        if (maybeRow instanceof Result.Err<?, ?> err) {
            @SuppressWarnings("unchecked")
            var error = (DbError) err.error();
            return new Result.Err<>(error);
        }

        @SuppressWarnings("unchecked")
        var ok = (Result.Ok<Option<Row>, DbError>) maybeRow;
        var option = ok.value();
        if (option instanceof Option.None<?>) {
            return Jdbc.ok(LumeRuntime.optionNone());
        }

        @SuppressWarnings("unchecked")
        var some = (Option.Some<Row>) option;
        var mapped = mapper.apply(some.value());
        if (mapped instanceof Result.Err<?, ?> err) {
            @SuppressWarnings("unchecked")
            var error = (DbError) err.error();
            return new Result.Err<>(error);
        }
        @SuppressWarnings("unchecked")
        var mappedOk = (Result.Ok<T, DbError>) mapped;
        return Jdbc.ok(LumeRuntime.optionSome(mappedOk.value()));
    }

    public <T> Result<Option<T>, DbError> mapValue(Function<Row, T> mapper) {
        var maybeRow = row();
        if (maybeRow instanceof Result.Err<?, ?> err) {
            @SuppressWarnings("unchecked")
            var error = (DbError) err.error();
            return new Result.Err<>(error);
        }

        @SuppressWarnings("unchecked")
        var ok = (Result.Ok<Option<Row>, DbError>) maybeRow;
        var option = ok.value();
        if (option instanceof Option.None<?>) {
            return Jdbc.ok(LumeRuntime.optionNone());
        }

        @SuppressWarnings("unchecked")
        var some = (Option.Some<Row>) option;
        return Jdbc.ok(LumeRuntime.optionSome(mapper.apply(some.value())));
    }
}
