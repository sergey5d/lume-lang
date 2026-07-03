package lume.db;

import java.sql.Connection;
import java.sql.SQLException;

import lume.core.LumeUnit;
import lume.core.Result;

public final class Transaction implements QueryRunner {
    private final Connection connection;
    private State state = State.OPEN;
    private DbError rollbackOnly;

    Transaction(Connection connection) {
        this.connection = connection;
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

    public Result<LumeUnit, DbError> commit() {
        if (state == State.COMMITTED) {
            return Jdbc.ok(LumeUnit.INSTANCE);
        }
        if (state == State.ROLLED_BACK) {
            return Jdbc.err("transaction has already been rolled back");
        }
        if (rollbackOnly != null) {
            rollback();
            return Jdbc.err("transaction is marked rollback-only: " + rollbackOnly.message());
        }
        try {
            connection.commit();
            closeConnection(State.COMMITTED);
            return Jdbc.ok(LumeUnit.INSTANCE);
        } catch (SQLException err) {
            return Jdbc.err(err);
        }
    }

    public Result<LumeUnit, DbError> rollback() {
        if (state == State.COMMITTED || state == State.ROLLED_BACK) {
            return Jdbc.ok(LumeUnit.INSTANCE);
        }
        try {
            connection.rollback();
            closeConnection(State.ROLLED_BACK);
            return Jdbc.ok(LumeUnit.INSTANCE);
        } catch (SQLException err) {
            return Jdbc.err(err);
        }
    }

    public Result<LumeUnit, DbError> close() {
        return rollback();
    }

    @Override
    public <T> Result<T, DbError> run(SqlWork<T> work) {
        if (state != State.OPEN) {
            return Jdbc.err("transaction is already closed");
        }
        if (rollbackOnly != null) {
            return Jdbc.err("transaction is marked rollback-only: " + rollbackOnly.message());
        }
        try {
            return Jdbc.ok(work.run(connection));
        } catch (SQLException err) {
            rollbackOnly = DbError.from(err);
            return new Result.Err<>(rollbackOnly);
        }
    }

    private void closeConnection(State finalState) throws SQLException {
        try {
            connection.close();
        } finally {
            state = finalState;
        }
    }

    private enum State {
        OPEN,
        COMMITTED,
        ROLLED_BACK,
    }
}
