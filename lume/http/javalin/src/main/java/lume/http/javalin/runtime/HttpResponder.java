package lume.http.javalin.runtime;

import io.javalin.http.Context;
import java.lang.reflect.InvocationTargetException;
import lume.core.LumeRuntime;
import lume.core.LumeType;
import lume.core.LumeUnit;
import lume.core.Option;
import lume.core.Result;

public final class HttpResponder {
    private static final String DEFAULT_SUCCESS_CONTENT_TYPE = "text/plain";
    private static final String DEFAULT_SHAPE_CONTENT_TYPE = "application/json";

    private HttpResponder() {
    }

    public static LumeUnit render(Context ctx, Object value, String defaultContentType) {
        if (value instanceof Result.Ok<?, ?> ok) {
            renderSuccess(ctx, ok.value(), defaultContentType);
            return LumeUnit.INSTANCE;
        }

        if (value instanceof Result.Err<?, ?> err) {
            renderError(ctx, err.error());
            return LumeUnit.INSTANCE;
        }

        renderSuccess(ctx, value, defaultContentType);
        return LumeUnit.INSTANCE;
    }

    private static void renderSuccess(Context ctx, Object value, String defaultContentType) {
        if (isLumeType(value, "HttpResponse")) {
            renderResponseShape(ctx, value, 200, contentTypeOr(defaultContentType, DEFAULT_SHAPE_CONTENT_TYPE));
            return;
        }

        ctx.status(200);
        ctx.contentType(contentTypeOr(defaultContentType, DEFAULT_SUCCESS_CONTENT_TYPE));
        ctx.result(String.valueOf(value));
    }

    private static void renderError(Context ctx, Object value) {
        if (isLumeType(value, "HttpError") || isLumeType(value, "HttpResponse")) {
            renderResponseShape(ctx, value, 500, DEFAULT_SHAPE_CONTENT_TYPE);
            return;
        }

        ctx.status(500);
        ctx.contentType(DEFAULT_SHAPE_CONTENT_TYPE);
        ctx.result(jsonError(String.valueOf(value)));
    }

    private static void renderResponseShape(Context ctx, Object value, int fallbackStatus, String fallbackContentType) {
        ctx.status(intField(value, "status", fallbackStatus));
        ctx.contentType(stringField(value, "contentType", fallbackContentType));
        ctx.result(stringField(value, "body", ""));
    }

    private static boolean isLumeType(Object value, String expectedName) {
        if (value == null) {
            return false;
        }

        LumeType runtimeType = LumeRuntime.runtimeTypeOf(value);
        Option<String> name = runtimeType.name();
        return name.isDefined() && expectedName.equals(LumeRuntime.extractSuccessValue(name));
    }

    private static int intField(Object value, String fieldName, int fallback) {
        Object fieldValue = field(value, fieldName);
        if (fieldValue == null) {
            return fallback;
        }
        if (fieldValue instanceof Number number) {
            return number.intValue();
        }
        if (fieldValue instanceof String text) {
            try {
                return Integer.parseInt(text);
            } catch (NumberFormatException ignored) {
                return fallback;
            }
        }
        return fallback;
    }

    private static String stringField(Object value, String fieldName, String fallback) {
        Object fieldValue = field(value, fieldName);
        if (fieldValue == null) {
            return fallback;
        }
        return String.valueOf(fieldValue);
    }

    private static Object field(Object value, String fieldName) {
        if (value == null) {
            return null;
        }

        try {
            var method = value.getClass().getMethod(fieldName);
            return method.invoke(value);
        } catch (NoSuchMethodException ignored) {
            // Shape/class fields are emitted as record accessors or Java fields depending on origin.
        } catch (IllegalAccessException | InvocationTargetException err) {
            throw new IllegalStateException("failed to read response field '" + fieldName + "'", err);
        }

        try {
            var field = value.getClass().getField(fieldName);
            return field.get(value);
        } catch (NoSuchFieldException ignored) {
            return null;
        } catch (IllegalAccessException err) {
            throw new IllegalStateException("failed to read response field '" + fieldName + "'", err);
        }
    }

    private static String contentTypeOr(String value, String fallback) {
        return value == null || value.isBlank() ? fallback : value;
    }

    private static String jsonError(String message) {
        return "{\"error\":" + jsonQuote(message) + "}";
    }

    private static String jsonQuote(String value) {
        if (value == null) {
            return "null";
        }
        return "\"" + escapeJson(value) + "\"";
    }

    private static String escapeJson(String value) {
        var out = new StringBuilder(value.length() + 16);
        for (var index = 0; index < value.length(); index++) {
            var ch = value.charAt(index);
            switch (ch) {
                case '"' -> out.append("\\\"");
                case '\\' -> out.append("\\\\");
                case '\b' -> out.append("\\b");
                case '\f' -> out.append("\\f");
                case '\n' -> out.append("\\n");
                case '\r' -> out.append("\\r");
                case '\t' -> out.append("\\t");
                default -> {
                    if (ch < 0x20) {
                        out.append(String.format("\\u%04x", (int) ch));
                    } else {
                        out.append(ch);
                    }
                }
            }
        }
        return out.toString();
    }
}
