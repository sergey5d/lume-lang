package lume.db;

import lume.core.LumeVector;
import lume.core.Result;

public interface JdbcRunner {
    Result<LumeVector<JdbcRow>, DbError> query(String sql, Object... values);

    Result<Long, DbError> exec(String sql);

    Result<Long, DbError> exec(String sql, Object... values);

    <T> Result<T, DbError> run(SqlWork<T> work);
}
