package lume.http.javalin.runtime;

import io.javalin.http.Context;
import lume.core.LumeRuntime;
import lume.core.Option;

public final class JavalinContext {
    private JavalinContext() {
    }

    public static Option<String> queryParam(Context ctx, String name) {
        return optional(ctx.queryParam(name));
    }

    public static Option<String> pathParam(Context ctx, String name) {
        return optional(ctx.pathParam(name));
    }

    public static Option<String> header(Context ctx, String name) {
        return optional(ctx.header(name));
    }

    public static Option<Long> queryInt(Context ctx, String name) {
        var raw = ctx.queryParam(name);
        if (raw == null || raw.isBlank()) {
            return LumeRuntime.optionNone();
        }
        return parseInt(raw);
    }

    public static Option<Long> parseInt(String raw) {
        try {
            return LumeRuntime.optionSome(Long.parseLong(raw));
        } catch (NumberFormatException err) {
            return LumeRuntime.optionNone();
        }
    }

    public static String jsonResponse(Context ctx, int status, String body) {
        ctx.status(status);
        ctx.contentType("application/json");
        ctx.result(body == null ? "null" : body);
        return body == null ? "null" : body;
    }

    private static Option<String> optional(String value) {
        if (value == null || value.isBlank()) {
            return LumeRuntime.optionNone();
        }
        return LumeRuntime.optionSome(value);
    }
}
