package lume.db;

import lume.core.Result;

public final class BoundExec {
    private final QueryRunner runner;
    private final String sql;
    private final SqlBindings bindings;

    BoundExec(QueryRunner runner, String sql, SqlBindings bindings) {
        this.runner = runner;
        this.sql = sql;
        this.bindings = bindings;
    }

    public Result<Long, DbError> run() {
        return runner.run(connection ->
            Jdbc.update(connection, sql, bindings.positional(), bindings.named())
        );
    }
}
