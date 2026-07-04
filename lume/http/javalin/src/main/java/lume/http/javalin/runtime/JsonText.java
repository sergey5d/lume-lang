package lume.http.javalin.runtime;

import lume.core.Option;
import lume.core.LumePanic;
import lume.core.LumeUnit;

public final class JsonText {
    private JsonText() {
    }

    public static String quote(String value) {
        if (value == null) {
            return "null";
        }
        return "\"" + escape(value) + "\"";
    }

    public static String stringOpt(Option<String> value) {
        return value.isDefined() ? quote(value.orPanic()) : "null";
    }

    public static String intOpt(Option<Long> value) {
        return value.isDefined() ? String.valueOf(value.orPanic()) : "null";
    }

    public static String error(String message) {
        return "{\"error\":" + quote(message) + "}";
    }

    public static String dbError(Object error) {
        return error("db error: " + String.valueOf(error));
    }

    public static String describe(Object value) {
        return String.valueOf(value);
    }

    public static LumeUnit panicDbConnect(Object error) {
        throw new LumePanic("failed to connect to DB: " + String.valueOf(error));
    }

    private static String escape(String value) {
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
