package lume.db;

import java.sql.Connection;
import java.sql.DriverManager;
import java.sql.SQLException;

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

    public Query query(String sql) {
        return new Query(this, sql);
    }

    public Result<Long, DbError> update(String sql, Object... values) {
        return query(sql).bind(values).update();
    }

    public Result<Long, DbError> insert(String sql, Object... values) {
        return query(sql).bind(values).insert();
    }

    public Result<Long, DbError> delete(String sql, Object... values) {
        return query(sql).bind(values).delete();
    }

    public Result<Transaction, DbError> begin() {
        try {
            var connection = openConnection();
            connection.setAutoCommit(false);
            return Jdbc.ok(new Transaction(connection));
        } catch (SQLException err) {
            return Jdbc.err(err);
        }
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
