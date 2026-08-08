package lume.db;

import lume.core.LumeArray;
import lume.core.Result;

public interface JdbcRunner {
    Result<LumeArray<JdbcRow>, DbError> query(String sql, Object... values);

    Result<Long, DbError> exec(String sql);

    Result<Long, DbError> exec(String sql, Object... values);

    <T> Result<T, DbError> run(SqlWork<T> work);
}
