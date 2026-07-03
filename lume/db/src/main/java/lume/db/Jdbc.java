package lume.db;

import java.sql.Connection;
import java.sql.PreparedStatement;
import java.sql.SQLException;
import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;

import lume.core.LumeList;
import lume.core.Result;

final class Jdbc {
    private Jdbc() {
    }

    static <T> Result<T, DbError> ok(T value) {
        return new Result.Ok<>(value);
    }

    static <T> Result<T, DbError> err(Throwable err) {
        return new Result.Err<>(DbError.from(err));
    }

    static <T> Result<T, DbError> err(String message) {
        return new Result.Err<>(DbError.message(message));
    }

    static LumeList<Row> query(
        Connection connection,
        String sql,
        List<Object> positional,
        Map<String, ?> named
    ) throws SQLException {
        var prepared = prepare(sql, positional, named);
        try (var statement = connection.prepareStatement(prepared.sql());
             var resultSet = bind(statement, prepared.values()).executeQuery()) {
            var metadata = resultSet.getMetaData();
            var columnCount = metadata.getColumnCount();
            var rows = new ArrayList<Row>();
            while (resultSet.next()) {
                var values = new LinkedHashMap<String, Object>();
                for (int index = 1; index <= columnCount; index++) {
                    var label = metadata.getColumnLabel(index);
                    if (label == null || label.isBlank()) {
                        label = metadata.getColumnName(index);
                    }
                    values.put(label, resultSet.getObject(index));
                }
                rows.add(new Row(values));
            }
            return LumeList.from(rows);
        }
    }

    static Long update(
        Connection connection,
        String sql,
        List<Object> positional,
        Map<String, ?> named
    ) throws SQLException {
        var prepared = prepare(sql, positional, named);
        try (var statement = connection.prepareStatement(prepared.sql())) {
            return (long) bind(statement, prepared.values()).executeUpdate();
        }
    }

    private static PreparedStatement bind(PreparedStatement statement, List<Object> values)
        throws SQLException {
        for (int index = 0; index < values.size(); index++) {
            statement.setObject(index + 1, values.get(index));
        }
        return statement;
    }

    private static PreparedSql prepare(String sql, List<Object> positional, Map<String, ?> named)
        throws SQLException {
        if (named == null) {
            return new PreparedSql(sql, positional == null ? List.of() : positional);
        }
        return prepareNamed(sql, named);
    }

    private static PreparedSql prepareNamed(String sql, Map<String, ?> named) throws SQLException {
        var out = new StringBuilder();
        var values = new ArrayList<Object>();
        var inSingleQuote = false;
        var inDoubleQuote = false;
        var inLineComment = false;
        var inBlockComment = false;

        for (int index = 0; index < sql.length(); index++) {
            char ch = sql.charAt(index);
            char next = index + 1 < sql.length() ? sql.charAt(index + 1) : '\0';

            if (inLineComment) {
                out.append(ch);
                if (ch == '\n') {
                    inLineComment = false;
                }
                continue;
            }

            if (inBlockComment) {
                out.append(ch);
                if (ch == '*' && next == '/') {
                    out.append(next);
                    index++;
                    inBlockComment = false;
                }
                continue;
            }

            if (!inSingleQuote && !inDoubleQuote && ch == '-' && next == '-') {
                out.append(ch).append(next);
                index++;
                inLineComment = true;
                continue;
            }

            if (!inSingleQuote && !inDoubleQuote && ch == '/' && next == '*') {
                out.append(ch).append(next);
                index++;
                inBlockComment = true;
                continue;
            }

            if (!inDoubleQuote && ch == '\'') {
                out.append(ch);
                inSingleQuote = !inSingleQuote;
                continue;
            }

            if (!inSingleQuote && ch == '"') {
                out.append(ch);
                inDoubleQuote = !inDoubleQuote;
                continue;
            }

            if (!inSingleQuote && !inDoubleQuote && ch == ':' && isIdentifierStart(next)) {
                int end = index + 2;
                while (end < sql.length() && isIdentifierPart(sql.charAt(end))) {
                    end++;
                }
                var name = sql.substring(index + 1, end);
                if (!named.containsKey(name)) {
                    throw new SQLException("missing SQL bind value for :" + name);
                }
                out.append('?');
                values.add(named.get(name));
                index = end - 1;
                continue;
            }

            out.append(ch);
        }

        return new PreparedSql(out.toString(), values);
    }

    private static boolean isIdentifierStart(char ch) {
        return Character.isLetter(ch) || ch == '_';
    }

    private static boolean isIdentifierPart(char ch) {
        return Character.isLetterOrDigit(ch) || ch == '_';
    }

    private record PreparedSql(String sql, List<Object> values) {
    }
}
