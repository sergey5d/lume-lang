package lume.db;

import lume.core.Result;

public final class Db {
    private Db() {
    }

    public static Result<Database, DbError> connect(String url) {
        var opened = JdbcDatabase.connect(url);
        if (opened instanceof Result.Err<?, ?> err) {
            @SuppressWarnings("unchecked")
            var error = (DbError) err.error();
            return new Result.Err<>(error);
        }

        @SuppressWarnings("unchecked")
        var ok = (Result.Ok<JdbcDatabase, DbError>) opened;
        return new Result.Ok<>(new Database(ok.value()));
    }

    public static Result<Database, DbError> connect(String url, String user, String password) {
        var opened = JdbcDatabase.connect(url, user, password);
        if (opened instanceof Result.Err<?, ?> err) {
            @SuppressWarnings("unchecked")
            var error = (DbError) err.error();
            return new Result.Err<>(error);
        }

        @SuppressWarnings("unchecked")
        var ok = (Result.Ok<JdbcDatabase, DbError>) opened;
        return new Result.Ok<>(new Database(ok.value()));
    }
}
