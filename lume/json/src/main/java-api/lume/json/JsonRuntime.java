package lume.json;

import lume.core.LumeVector;
import lume.core.LumeType;
import lume.core.Result;

public final class JsonRuntime {
    private JsonRuntime() {
    }

    public static JsonValue str(String value) {
        throw new UnsupportedOperationException("descriptor only");
    }

    public static JsonValue int_(Long value) {
        throw new UnsupportedOperationException("descriptor only");
    }

    public static JsonValue float_(Double value) {
        throw new UnsupportedOperationException("descriptor only");
    }

    public static JsonValue bool(Boolean value) {
        throw new UnsupportedOperationException("descriptor only");
    }

    public static JsonValue nil() {
        throw new UnsupportedOperationException("descriptor only");
    }

    public static JsonField field(String name, JsonValue value) {
        throw new UnsupportedOperationException("descriptor only");
    }

    public static JsonValue array(LumeVector<JsonValue> values) {
        throw new UnsupportedOperationException("descriptor only");
    }

    public static JsonValue obj(LumeVector<JsonField> fields) {
        throw new UnsupportedOperationException("descriptor only");
    }

    public static JsonValue encode(Object value) {
        throw new UnsupportedOperationException("descriptor only");
    }

    public static String stringify(Object value) {
        throw new UnsupportedOperationException("descriptor only");
    }

    public static <T> Result<T, String> decode(String text, LumeType targetType) {
        throw new UnsupportedOperationException("descriptor only");
    }
}
