package lume.db;

import java.sql.Connection;
import java.sql.DriverManager;
import java.sql.SQLException;
import java.util.function.Function;

import lume.core.LumeUnit;
import lume.core.Result;

public final class Database implements QueryRunner {
    private final String url;
    private final String user;
    private final String password;
    private final Boolean hasCredentials;

    private Database(String url, String user, String password, Boolean hasCredentials) {
        this.url = url;
        this.user = user;
        this.password = password;
        this.hasCredentials = hasCredentials;
    }

    public static Result<Database, DbError> connect(String url) {
        return create(url, "", "", false);
    }

    public static Result<Database, DbError> connect(String url, String user, String password) {
        return create(url, user, password, true);
    }

    private static Result<Database, DbError> create(
        String url,
        String user,
        String password,
        Boolean hasCredentials
    ) {
        var database = new Database(url, user, password, hasCredentials);
        try (var ignored = database.openConnection()) {
            return Jdbc.ok(database);
        } catch (SQLException err) {
            return Jdbc.err(err);
        }
    }

    public Query query(String sql, Object... values) {
        return new Query(this, sql).bind(values);
    }

    public RowQuery queryRow(String sql, Object... values) {
        return new RowQuery(this, sql).bind(values);
    }

    public Exec exec(String sql) {
        return new Exec(this, sql);
    }

    public Result<Long, DbError> exec(String sql, Object first, Object... rest) {
        return new BoundExec(this, sql, SqlBindings.from(first, rest)).run();
    }

    public Result<Transaction, DbError> beginTransaction() {
        try {
            var connection = openConnection();
            connection.setAutoCommit(false);
            return Jdbc.ok(new Transaction(connection));
        } catch (SQLException err) {
            return Jdbc.err(err);
        }
    }

    public Result<Transaction, DbError> begin() {
        return beginTransaction();
    }

    public <T> Result<T, DbError> transaction(Function<Transaction, Result<T, DbError>> work) {
        var opened = beginTransaction();
        if (opened instanceof Result.Err<?, ?> err) {
            @SuppressWarnings("unchecked")
            var error = (DbError) err.error();
            return new Result.Err<>(error);
        }

        @SuppressWarnings("unchecked")
        var ok = (Result.Ok<Transaction, DbError>) opened;
        var tx = ok.value();
        try {
            var result = work.apply(tx);
            if (result instanceof Result.Err<?, ?>) {
                tx.rollback();
                return result;
            }

            var committed = tx.commit();
            if (committed instanceof Result.Err<?, ?> err) {
                @SuppressWarnings("unchecked")
                var error = (DbError) err.error();
                return new Result.Err<>(error);
            }
            return result;
        } catch (RuntimeException err) {
            tx.rollback();
            throw err;
        }
    }

    public Result<LumeUnit, DbError> transactionally(
        Function<Transaction, Result<LumeUnit, DbError>> work
    ) {
        return transaction(work);
    }

    public Result<LumeUnit, DbError> close() {
        return Jdbc.ok(LumeUnit.INSTANCE);
    }

    Connection openConnection() throws SQLException {
        if (hasCredentials) {
            return DriverManager.getConnection(url, user, password);
        }
        return DriverManager.getConnection(url);
    }

    @Override
    public <T> Result<T, DbError> run(SqlWork<T> work) {
        try (var connection = openConnection()) {
            return Jdbc.ok(work.run(connection));
        } catch (SQLException err) {
            return Jdbc.err(err);
        }
    }
}
