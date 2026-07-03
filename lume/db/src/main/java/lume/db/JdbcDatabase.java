package lume.db;

import java.sql.Connection;
import java.sql.DriverManager;
import java.sql.SQLException;

import lume.core.LumeList;
import lume.core.LumeUnit;
import lume.core.Result;

public final class JdbcDatabase implements JdbcRunner {
    private final String url;
    private final String user;
    private final String password;
    private final Boolean hasCredentials;

    private JdbcDatabase(String url, String user, String password, Boolean hasCredentials) {
        this.url = url;
        this.user = user;
        this.password = password;
        this.hasCredentials = hasCredentials;
    }

    public static Result<JdbcDatabase, DbError> connect(String url) {
        return create(url, "", "", false);
    }

    public static Result<JdbcDatabase, DbError> connect(String url, String user, String password) {
        return create(url, user, password, true);
    }

    private static Result<JdbcDatabase, DbError> create(
        String url,
        String user,
        String password,
        Boolean hasCredentials
    ) {
        var database = new JdbcDatabase(url, user, password, hasCredentials);
        try (var ignored = database.openConnection()) {
            return Jdbc.ok(database);
        } catch (SQLException err) {
            return Jdbc.err(err);
        }
    }

    @Override
    public Result<LumeList<JdbcRow>, DbError> query(String sql, Object... values) {
        var bindings = SqlBindings.from(values);
        return run(connection ->
            Jdbc.query(connection, sql, bindings.positional(), bindings.named())
        );
    }

    @Override
    public Result<Long, DbError> exec(String sql) {
        return exec(sql, SqlBindings.empty());
    }

    @Override
    public Result<Long, DbError> exec(String sql, Object... values) {
        return exec(sql, SqlBindings.from(values));
    }

    private Result<Long, DbError> exec(String sql, SqlBindings bindings) {
        return run(connection ->
            Jdbc.update(connection, sql, bindings.positional(), bindings.named())
        );
    }

    public Result<JdbcTransaction, DbError> beginTransaction() {
        try {
            var connection = openConnection();
            connection.setAutoCommit(false);
            return Jdbc.ok(new JdbcTransaction(connection));
        } catch (SQLException err) {
            return Jdbc.err(err);
        }
    }

    public Result<JdbcTransaction, DbError> begin() {
        return beginTransaction();
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
