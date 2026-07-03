package lume.db;

import java.sql.SQLException;

import lume.core.LumeRuntime;
import lume.core.Option;

public final class DbError {
    private final String message;
    private final String sqlState;
    private final Integer vendorCode;

    public DbError(String message) {
        this(message, null, 0);
    }

    public DbError(String message, String sqlState, Integer vendorCode) {
        this.message = message == null ? "database error" : message;
        this.sqlState = sqlState;
        this.vendorCode = vendorCode == null ? 0 : vendorCode;
    }

    public String message() {
        return message;
    }

    public Option<String> sqlState() {
        if (sqlState == null || sqlState.isBlank()) {
            return LumeRuntime.optionNone();
        }
        return LumeRuntime.optionSome(sqlState);
    }

    public Integer vendorCode() {
        return vendorCode;
    }

    @Override
    public String toString() {
        if (sqlState == null || sqlState.isBlank()) {
            return message;
        }
        return message + " (SQL state " + sqlState + ", vendor code " + vendorCode + ")";
    }

    public static DbError from(Throwable err) {
        if (err instanceof SQLException sql) {
            return new DbError(sql.getMessage(), sql.getSQLState(), sql.getErrorCode());
        }
        return new DbError(err.getMessage());
    }

    public static DbError message(String message) {
        return new DbError(message);
    }
}
