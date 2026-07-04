package lume.json;

import lume.core.LumeArray;
import lume.core.LumeField;
import lume.core.LumeList;
import lume.core.LumeMap;
import lume.core.LumeRuntime;
import lume.core.LumeSet;
import lume.core.LumeType;
import lume.core.LumeTypeKind;
import lume.core.Option;

import java.lang.reflect.InvocationTargetException;
import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.concurrent.ConcurrentHashMap;

public final class JsonRuntime {
    private static final ConcurrentHashMap<LumeType, List<EncodedField>> ENCODER_CACHE =
            new ConcurrentHashMap<>();

    private JsonRuntime() {
    }

    public static JsonValue str(String value) {
        return new JsonValue.JsonString(value);
    }

    public static JsonValue int_(Long value) {
        return number(value);
    }

    public static JsonValue float_(Double value) {
        return number(value);
    }

    public static JsonValue bool(Boolean value) {
        return new JsonValue.JsonBool(value);
    }

    public static JsonValue nil() {
        return new JsonValue.JsonNull();
    }

    public static JsonField field(String name, JsonValue value) {
        return new JsonField(name, value);
    }

    public static JsonValue obj(LumeList<JsonField> fields) {
        return new JsonValue.JsonObject(fields.asJava());
    }

    public static JsonValue array(LumeList<JsonValue> values) {
        return new JsonValue.JsonArray(values.asJava());
    }

    public static JsonValue encode(Object value) {
        if (value instanceof JsonValue json) {
            return json;
        }
        if (value == null) {
            return nil();
        }
        if (value instanceof String text) {
            return str(text);
        }
        if (value instanceof Boolean bool) {
            return bool(bool);
        }
        if (value instanceof Long
                || value instanceof Integer
                || value instanceof Short
                || value instanceof Byte
                || value instanceof Double
                || value instanceof Float) {
            return number((Number) value);
        }
        if (value instanceof Option<?> option) {
            return option.isDefined() ? encode(option.orPanic()) : nil();
        }
        if (value instanceof LumeList<?> list) {
            return encodeIterable(list.asJava());
        }
        if (value instanceof LumeArray<?> array) {
            return encodeIterable(array.asJava());
        }
        if (value instanceof LumeSet<?> set) {
            return encodeIterable(set.asJava());
        }
        if (value instanceof LumeMap<?, ?> map) {
            return encodeMap(map.asJava());
        }
        if (value instanceof Iterable<?> iterable) {
            return encodeIterable(iterable);
        }
        if (value instanceof Map<?, ?> map) {
            return encodeMap(map);
        }
        if (value.getClass().isArray()) {
            return encodeJavaArray(value);
        }

        var type = LumeRuntime.runtimeTypeOf(value);
        if (type.kind() == LumeTypeKind.Class
                || type.kind() == LumeTypeKind.Shape
                || type.kind() == LumeTypeKind.Single) {
            return encodeStructured(value, type);
        }
        if (type.kind() == LumeTypeKind.Enum) {
            return encodeEnum(value);
        }
        return str(String.valueOf(value));
    }

    public static String stringify(Object value) {
        return encode(value).stringify();
    }

    static String escape(String value) {
        if (value == null) {
            return "";
        }
        var out = new StringBuilder();
        for (int index = 0; index < value.length(); index++) {
            char ch = value.charAt(index);
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

    private static JsonValue number(Number value) {
        if (value == null) {
            return nil();
        }
        return new JsonValue.JsonNumber(value.toString());
    }

    private static JsonValue encodeIterable(Iterable<?> values) {
        var out = new ArrayList<JsonValue>();
        for (var value : values) {
            out.add(encode(value));
        }
        return new JsonValue.JsonArray(out);
    }

    private static JsonValue encodeJavaArray(Object array) {
        var length = java.lang.reflect.Array.getLength(array);
        var out = new ArrayList<JsonValue>();
        for (int index = 0; index < length; index++) {
            out.add(encode(java.lang.reflect.Array.get(array, index)));
        }
        return new JsonValue.JsonArray(out);
    }

    private static JsonValue encodeMap(Map<?, ?> map) {
        var fields = new ArrayList<JsonField>();
        for (var entry : map.entrySet()) {
            fields.add(field(String.valueOf(entry.getKey()), encode(entry.getValue())));
        }
        return new JsonValue.JsonObject(fields);
    }

    private static JsonValue encodeStructured(Object receiver, LumeType type) {
        var fields = new ArrayList<JsonField>();
        for (var encodedField : ENCODER_CACHE.computeIfAbsent(type, JsonRuntime::buildEncoderFields)) {
            fields.add(field(encodedField.jsonName(), encode(readField(encodedField.field(), receiver))));
        }
        return new JsonValue.JsonObject(fields);
    }

    private static List<EncodedField> buildEncoderFields(LumeType type) {
        var out = new ArrayList<EncodedField>();
        for (LumeField field : type.fields().asJava()) {
            if (field.isHidden() || field.hasAnnotation("JsonIgnore")) {
                continue;
            }
            out.add(new EncodedField(jsonFieldName(field), field));
        }
        return List.copyOf(out);
    }

    private static String jsonFieldName(LumeField field) {
        var annotation = field.annotation("JsonName");
        if (annotation.isDefined()) {
            var value = annotation.orPanic().str("value");
            if (value.isDefined() && !value.orPanic().isBlank()) {
                return value.orPanic();
            }
        }
        return field.name();
    }

    private static Object readField(LumeField field, Object receiver) {
        var result = field.get(receiver);
        if (result.isOk()) {
            return result.orPanic();
        }
        throw new IllegalStateException(result.getError().message());
    }

    private static JsonValue encodeEnum(Object value) {
        var caseName = value.getClass().getSimpleName();
        var fields = new LinkedHashMap<String, Object>();
        for (var method : value.getClass().getDeclaredMethods()) {
            if (method.getParameterCount() != 0 || method.isSynthetic()) {
                continue;
            }
            try {
                method.setAccessible(true);
                fields.put(method.getName(), method.invoke(value));
            } catch (IllegalAccessException | InvocationTargetException err) {
                throw new IllegalStateException("cannot encode enum case field '" + method.getName() + "'", err);
            }
        }
        if (fields.isEmpty()) {
            return str(caseName);
        }
        return new JsonValue.JsonObject(List.of(field(caseName, encodeMap(fields))));
    }

    private record EncodedField(String jsonName, LumeField field) {
    }
}
