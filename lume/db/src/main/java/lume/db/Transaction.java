package lume.db;

import java.sql.Connection;
import java.sql.SQLException;

import lume.core.LumeUnit;
import lume.core.Result;

public final class Transaction implements QueryRunner {
    private final Connection connection;
    private Boolean closed = false;

    Transaction(Connection connection) {
        this.connection = connection;
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

    public Result<LumeUnit, DbError> commit() {
        if (closed) {
            return Jdbc.err("transaction is already closed");
        }
        try {
            connection.commit();
            closeConnection();
            return Jdbc.ok(LumeUnit.INSTANCE);
        } catch (SQLException err) {
            return Jdbc.err(err);
        }
    }

    public Result<LumeUnit, DbError> rollback() {
        if (closed) {
            return Jdbc.err("transaction is already closed");
        }
        try {
            connection.rollback();
            closeConnection();
            return Jdbc.ok(LumeUnit.INSTANCE);
        } catch (SQLException err) {
            return Jdbc.err(err);
        }
    }

    public Result<LumeUnit, DbError> close() {
        if (closed) {
            return Jdbc.ok(LumeUnit.INSTANCE);
        }
        try {
            connection.rollback();
            closeConnection();
            return Jdbc.ok(LumeUnit.INSTANCE);
        } catch (SQLException err) {
            return Jdbc.err(err);
        }
    }

    @Override
    public <T> Result<T, DbError> run(SqlWork<T> work) {
        if (closed) {
            return Jdbc.err("transaction is already closed");
        }
        try {
            return Jdbc.ok(work.run(connection));
        } catch (SQLException err) {
            return Jdbc.err(err);
        }
    }

    private void closeConnection() throws SQLException {
        try {
            connection.close();
        } finally {
            closed = true;
        }
    }
}
