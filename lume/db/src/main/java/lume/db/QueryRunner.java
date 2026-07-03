package lume.db;

import lume.core.Result;

interface QueryRunner {
    <T> Result<T, DbError> run(SqlWork<T> work);
}
