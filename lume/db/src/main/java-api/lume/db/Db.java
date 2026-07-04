package lume.db;

import java.net.URI;
import java.util.ArrayList;
import java.util.List;

import lume.core.Result;

public final class Db {
    private Db() {
    }

    public static Result<Database, DbError> connect(String url) {
        var opened = JdbcDatabase.connect(jdbcUrl(url));
        if (opened instanceof Result.Err<?, ?> err) {
            @SuppressWarnings("unchecked")
            var error = (DbError) err.error();
            return new Result.Err<>(error);
        }

        @SuppressWarnings("unchecked")
        var ok = (Result.Ok<JdbcDatabase, DbError>) opened;
        return new Result.Ok<>(new JdbcDatabaseImpl(ok.value()));
    }

    public static Result<Database, DbError> connect(String url, String user, String password) {
        var opened = JdbcDatabase.connect(jdbcUrl(url), user, password);
        if (opened instanceof Result.Err<?, ?> err) {
            @SuppressWarnings("unchecked")
            var error = (DbError) err.error();
            return new Result.Err<>(error);
        }

        @SuppressWarnings("unchecked")
        var ok = (Result.Ok<JdbcDatabase, DbError>) opened;
        return new Result.Ok<>(new JdbcDatabaseImpl(ok.value()));
    }

    private static String jdbcUrl(String url) {
        if (url == null) {
            return null;
        }
        if (url.startsWith("jdbc:")) {
            return url;
        }
        if (url.startsWith("postgres://")) {
            return postgresUrlToJdbc(url);
        }
        if (url.startsWith("postgresql://")) {
            return postgresUrlToJdbc(url);
        }
        return url;
    }

    private static String postgresUrlToJdbc(String url) {
        var uri = URI.create(url);
        var out = new StringBuilder("jdbc:postgresql://");

        var host = uri.getHost();
        if (host == null || host.isBlank()) {
            return "jdbc:postgresql://" + stripPostgresScheme(url);
        }

        out.append(host);
        if (uri.getPort() != -1) {
            out.append(':').append(uri.getPort());
        }

        var path = uri.getRawPath();
        out.append(path == null || path.isBlank() ? "/" : path);

        var params = new ArrayList<String>();
        var query = uri.getRawQuery();
        if (query != null && !query.isBlank()) {
            params.add(query);
        }

        var userInfo = uri.getRawUserInfo();
        if (userInfo != null && !userInfo.isBlank()) {
            var separator = userInfo.indexOf(':');
            if (separator < 0) {
                params.add("user=" + userInfo);
            } else {
                params.add("user=" + userInfo.substring(0, separator));
                params.add("password=" + userInfo.substring(separator + 1));
            }
        }

        if (!params.isEmpty()) {
            out.append('?').append(String.join("&", params));
        }

        return out.toString();
    }

    private static String stripPostgresScheme(String url) {
        for (var scheme : List.of("postgres://", "postgresql://")) {
            if (url.startsWith(scheme)) {
                return url.substring(scheme.length());
            }
        }
        return url;
    }
}
