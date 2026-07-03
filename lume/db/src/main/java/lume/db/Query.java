package lume.db;

import java.util.ArrayList;
import java.util.List;
import java.util.Map;
import java.util.function.Function;

import lume.core.LumeList;
import lume.core.LumeMap;
import lume.core.Option;
import lume.core.Result;
import lume.core.Tuple2;
import lume.core.Tuple3;
import lume.core.Tuple4;
import lume.core.Tuple5;
import lume.core.Tuple6;
import lume.core.Tuple7;
import lume.core.Tuple8;

public final class Query {
    private final QueryRunner runner;
    private final String sql;
    private final List<Object> positional;
    private final Map<String, ?> named;

    Query(QueryRunner runner, String sql) {
        this(runner, sql, List.of(), null);
    }

    private Query(QueryRunner runner, String sql, List<Object> positional, Map<String, ?> named) {
        this.runner = runner;
        this.sql = sql;
        this.positional = List.copyOf(positional);
        this.named = named;
    }

    public Query bind(Object... values) {
        return new Query(runner, sql, List.of(values), null);
    }

    public Query bindAll(LumeList<?> values) {
        return new Query(runner, sql, new ArrayList<>(values.asJava()), null);
    }

    public Query bindTuple(Tuple2<?, ?> values) {
        return bind(values.first(), values.second());
    }

    public Query bindTuple3(Tuple3<?, ?, ?> values) {
        return bind(values.first(), values.second(), values.third());
    }

    public Query bindTuple4(Tuple4<?, ?, ?, ?> values) {
        return bind(values.first(), values.second(), values.third(), values.fourth());
    }

    public Query bindTuple5(Tuple5<?, ?, ?, ?, ?> values) {
        return bind(values.first(), values.second(), values.third(), values.fourth(), values.fifth());
    }

    public Query bindTuple6(Tuple6<?, ?, ?, ?, ?, ?> values) {
        return bind(
            values.first(),
            values.second(),
            values.third(),
            values.fourth(),
            values.fifth(),
            values.sixth()
        );
    }

    public Query bindTuple7(Tuple7<?, ?, ?, ?, ?, ?, ?> values) {
        return bind(
            values.first(),
            values.second(),
            values.third(),
            values.fourth(),
            values.fifth(),
            values.sixth(),
            values.seventh()
        );
    }

    public Query bindTuple8(Tuple8<?, ?, ?, ?, ?, ?, ?, ?> values) {
        return bind(
            values.first(),
            values.second(),
            values.third(),
            values.fourth(),
            values.fifth(),
            values.sixth(),
            values.seventh(),
            values.eighth()
        );
    }

    public Query bindNamed(LumeMap<String, ?> values) {
        return new Query(runner, sql, List.of(), values.asJava());
    }

    public Result<LumeList<Row>, DbError> all() {
        return runner.run(connection -> Jdbc.query(connection, sql, positional, named));
    }

    public Result<Option<Row>, DbError> first() {
        var rows = all();
        if (rows instanceof Result.Err<?, ?> err) {
            @SuppressWarnings("unchecked")
            var error = (DbError) err.error();
            return new Result.Err<>(error);
        }
        @SuppressWarnings("unchecked")
        var ok = (Result.Ok<LumeList<Row>, DbError>) rows;
        return new Result.Ok<>(ok.value().get(0));
    }

    public <T> Result<LumeList<T>, DbError> map(Function<Row, Result<T, DbError>> mapper) {
        var rows = all();
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
        var rows = all();
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

    public Result<Long, DbError> update() {
        return runner.run(connection -> Jdbc.update(connection, sql, positional, named));
    }

    public Result<Long, DbError> insert() {
        return update();
    }

    public Result<Long, DbError> delete() {
        return update();
    }
}
