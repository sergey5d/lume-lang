package lume.db;

public final class Exec {
    private final QueryRunner runner;
    private final String sql;

    Exec(QueryRunner runner, String sql) {
        this.runner = runner;
        this.sql = sql;
    }

    public BoundExec bind(Object... values) {
        return new BoundExec(runner, sql, SqlBindings.from(values));
    }

    public BoundExec bind() {
        return new BoundExec(runner, sql, SqlBindings.empty());
    }

    public lume.core.Result<Long, DbError> run() {
        return bind().run();
    }
}
