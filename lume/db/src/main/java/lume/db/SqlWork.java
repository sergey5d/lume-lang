package lume.db;

import java.sql.Connection;
import java.sql.SQLException;

@FunctionalInterface
interface SqlWork<T> {
    T run(Connection connection) throws SQLException;
}
