package lume.db;

import java.sql.Connection;
import java.sql.SQLException;

import com.zaxxer.hikari.HikariConfig;
import com.zaxxer.hikari.HikariDataSource;

import lume.core.LumeVector;
import lume.core.LumeUnit;
import lume.core.Result;

public final class JdbcDatabase implements JdbcRunner {
    private final HikariDataSource dataSource;

    private JdbcDatabase(HikariDataSource dataSource) {
        this.dataSource = dataSource;
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
        var config = new HikariConfig();
        config.setJdbcUrl(url);
        config.setPoolName("lume-db");
        if (hasCredentials) {
            config.setUsername(user);
            config.setPassword(password);
        }

        HikariDataSource dataSource = null;
        try {
            dataSource = new HikariDataSource(config);
            var database = new JdbcDatabase(dataSource);
            try (var ignored = dataSource.getConnection()) {
                return Jdbc.ok(database);
            }
        } catch (SQLException err) {
            closeQuietly(dataSource);
            return Jdbc.err(err);
        } catch (RuntimeException err) {
            closeQuietly(dataSource);
            return new Result.Err<>(DbError.from(err));
        }
    }

    private static void closeQuietly(HikariDataSource dataSource) {
        if (dataSource != null) {
            dataSource.close();
        }
    }

    @Override
    public Result<LumeVector<JdbcRow>, DbError> query(String sql, Object... values) {
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
        dataSource.close();
        return Jdbc.ok(LumeUnit.INSTANCE);
    }

    Connection openConnection() throws SQLException {
        return dataSource.getConnection();
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
