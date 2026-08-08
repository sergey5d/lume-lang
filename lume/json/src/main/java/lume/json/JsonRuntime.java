package lume.json;

import lume.core.LumeFixedArray;
import lume.core.LumeField;
import lume.core.LumeArray;
import lume.core.LumeMap;
import lume.core.LumeRuntime;
import lume.core.LumeSet;
import lume.core.LumeType;
import lume.core.LumeTypeKind;
import lume.core.Option;
import lume.core.Result;

import java.lang.reflect.Constructor;
import java.lang.reflect.Field;
import java.lang.reflect.InvocationTargetException;
import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.concurrent.ConcurrentHashMap;

public final class JsonRuntime {
    private static final ConcurrentHashMap<LumeType, List<EncodedField>> ENCODER_CACHE =
            new ConcurrentHashMap<>();
    private static final Object NO_DEFAULT = new Object();

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

    public static JsonValue obj(LumeArray<JsonField> fields) {
        return new JsonValue.JsonObject(fields);
    }

    public static JsonValue array(LumeArray<JsonValue> values) {
        return new JsonValue.JsonArray(values);
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
            return option.isDefined() ? encode(LumeRuntime.extractSuccessValue(option)) : nil();
        }
        if (value instanceof LumeArray<?> list) {
            return encodeIterable(list.asJava());
        }
        if (value instanceof LumeFixedArray<?> array) {
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
                || type.kind() == LumeTypeKind.Object) {
            return encodeStructured(value, type);
        }
        if (type.kind() == LumeTypeKind.Enum) {
            return encodeEnum(value);
        }
        return str(String.valueOf(value));
    }

    public static String stringify(Object value) {
        return render(encode(value));
    }

    public static <T> Result<T, String> decode(String text, LumeType targetType) {
        try {
            var parser = new Parser(text == null ? "" : text);
            var parsed = parser.parse();
            @SuppressWarnings("unchecked")
            var decoded = (T) decodeValue(parsed, targetType, packageName(targetType));
            return new Result.Ok<>(decoded);
        } catch (DecodeFailure err) {
            return new Result.Err<>(err.getMessage());
        } catch (RuntimeException err) {
            var message = err.getMessage() == null ? err.getClass().getSimpleName() : err.getMessage();
            return new Result.Err<>(message);
        }
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
        return new JsonValue.JsonArray(LumeArray.from(out));
    }

    private static JsonValue encodeJavaArray(Object array) {
        var length = java.lang.reflect.Array.getLength(array);
        var out = new ArrayList<JsonValue>();
        for (int index = 0; index < length; index++) {
            out.add(encode(java.lang.reflect.Array.get(array, index)));
        }
        return new JsonValue.JsonArray(LumeArray.from(out));
    }

    private static JsonValue encodeMap(Map<?, ?> map) {
        var fields = new ArrayList<JsonField>();
        for (var entry : map.entrySet()) {
            fields.add(field(String.valueOf(entry.getKey()), encode(entry.getValue())));
        }
        return new JsonValue.JsonObject(LumeArray.from(fields));
    }

    private static JsonValue encodeStructured(Object receiver, LumeType type) {
        var fields = new ArrayList<JsonField>();
        for (var encodedField : ENCODER_CACHE.computeIfAbsent(type, JsonRuntime::buildEncoderFields)) {
            fields.add(field(encodedField.jsonName(), encode(readField(encodedField.field(), receiver))));
        }
        return new JsonValue.JsonObject(LumeArray.from(fields));
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
            var value = ((lume.core.LumeAnnotation) LumeRuntime.extractSuccessValue(annotation))
                    .str("value");
            if (value.isDefined()) {
                var name = (String) LumeRuntime.extractSuccessValue(value);
                if (!name.isBlank()) {
                    return name;
                }
            }
        }
        return field.name();
    }

    private static Object readField(LumeField field, Object receiver) {
        var result = field.get(receiver);
        if (result.isOk()) {
            return LumeRuntime.extractSuccessValue(result);
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
        return new JsonValue.JsonObject(LumeArray.of(field(caseName, encodeMap(fields))));
    }

    private static String render(JsonValue value) {
        if (value instanceof JsonValue.JsonNull) {
            return "null";
        }
        if (value instanceof JsonValue.JsonBool json) {
            return Boolean.TRUE.equals(json.value()) ? "true" : "false";
        }
        if (value instanceof JsonValue.JsonNumber json) {
            return json.value();
        }
        if (value instanceof JsonValue.JsonString json) {
            return "\"" + escape(json.value()) + "\"";
        }
        if (value instanceof JsonValue.JsonArray json) {
            var out = new StringBuilder();
            out.append("[");
            var values = json.values().asJava();
            for (int index = 0; index < values.size(); index++) {
                if (index > 0) {
                    out.append(",");
                }
                out.append(render(values.get(index)));
            }
            out.append("]");
            return out.toString();
        }
        if (value instanceof JsonValue.JsonObject json) {
            var out = new StringBuilder();
            out.append("{");
            var fields = json.fields().asJava();
            for (int index = 0; index < fields.size(); index++) {
                if (index > 0) {
                    out.append(",");
                }
                var field = fields.get(index);
                out.append("\"").append(escape(field.name())).append("\":");
                out.append(render(field.value()));
            }
            out.append("}");
            return out.toString();
        }
        return "\"" + escape(String.valueOf(value)) + "\"";
    }

    private record EncodedField(String jsonName, LumeField field) {
    }

    private static Object decodeValue(Object value, LumeType targetType, String contextPackage) {
        return decodeByDescriptor(value, typeName(targetType), targetType, contextPackage);
    }

    private static Object decodeByDescriptor(
            Object value,
            String descriptor,
            LumeType targetType,
            String contextPackage) {
        var trimmed = descriptor.trim();
        if (isOptionDescriptor(trimmed)) {
            if (value == null) {
                return LumeRuntime.optionNone();
            }
            return LumeRuntime.optionSome(decodeByDescriptor(
                    value,
                    innerDescriptor(trimmed, "Option"),
                    null,
                    contextPackage));
        }
        if (isArrayDescriptor(trimmed)) {
            if (!(value instanceof List<?> list)) {
                throw new DecodeFailure("expected JSON array for " + trimmed);
            }
            var inner = arrayInnerDescriptor(trimmed);
            var out = new ArrayList<>();
            for (var item : list) {
                out.add(decodeByDescriptor(item, inner, null, contextPackage));
            }
            return LumeArray.from(out);
        }
        if (targetType != null
                && (targetType.kind() == LumeTypeKind.Class || targetType.kind() == LumeTypeKind.Shape)) {
            return decodeStructured(value, targetType);
        }
        return switch (trimmed) {
            case "Any" -> value;
            case "Str" -> value == null ? "" : String.valueOf(value);
            case "Int", "Int64" -> toLong(value, trimmed);
            case "Rune" -> Math.toIntExact(toLong(value, trimmed));
            case "Float", "Float64" -> toDouble(value, trimmed);
            case "Bool" -> toBool(value, trimmed);
            default -> decodeStructured(value, loadType(trimmed, contextPackage));
        };
    }

    private static Object decodeStructured(Object value, LumeType targetType) {
        if (!(value instanceof Map<?, ?> rawMap)) {
            throw new DecodeFailure("expected JSON object for " + typeLabel(targetType));
        }

        var map = new LinkedHashMap<String, Object>();
        for (var entry : rawMap.entrySet()) {
            map.put(String.valueOf(entry.getKey()), entry.getValue());
        }

        var qualified = targetType.qualifiedName();
        if (!qualified.isDefined()) {
            throw new DecodeFailure("cannot decode " + typeLabel(targetType) + ": type has no Java qualified name");
        }

        var javaName = (String) LumeRuntime.extractSuccessValue(qualified);
        Class<?> targetClass;
        try {
            targetClass = Class.forName(javaName);
        } catch (ClassNotFoundException err) {
            throw new DecodeFailure("cannot decode " + typeLabel(targetType) + ": Java class is not available");
        }

        var fields = targetType.fields().asJava().stream()
                .filter(field -> !field.isHidden())
                .toList();
        var args = new Object[fields.size()];
        var packageName = packageName(targetType);

        for (int index = 0; index < fields.size(); index++) {
            var field = fields.get(index);
            var jsonName = jsonFieldName(field);
            var descriptor = typeName(field.fieldType());
            if (!map.containsKey(jsonName)) {
                if (isOptionDescriptor(descriptor)) {
                    args[index] = LumeRuntime.optionNone();
                } else {
                    var defaultValue = defaultForDescriptor(descriptor);
                    if (defaultValue == NO_DEFAULT) {
                        throw new DecodeFailure("missing JSON field '" + jsonName + "' for " + typeLabel(targetType));
                    }
                    args[index] = defaultValue;
                }
            } else {
                var raw = map.get(jsonName);
                if (raw == null && !isOptionDescriptor(descriptor)) {
                    var defaultValue = defaultForDescriptor(descriptor);
                    if (defaultValue == NO_DEFAULT) {
                        throw new DecodeFailure("JSON field '" + jsonName + "' is null for non-optional field");
                    }
                    args[index] = defaultValue;
                } else {
                    args[index] = decodeByDescriptor(raw, descriptor, field.fieldType(), packageName);
                }
            }
        }

        Constructor<?> lastConstructor = null;
        RuntimeException lastFailure = null;
        for (var constructor : targetClass.getConstructors()) {
            if (constructor.getParameterCount() != args.length) {
                continue;
            }
            lastConstructor = constructor;
            try {
                return constructor.newInstance(convertArgs(constructor.getParameterTypes(), args));
            } catch (ReflectiveOperationException | RuntimeException err) {
                lastFailure = new DecodeFailure(reflectionMessage(err));
            }
        }

        if (lastFailure != null) {
            throw new DecodeFailure("cannot construct " + typeLabel(targetType) + ": " + lastFailure.getMessage());
        }
        throw new DecodeFailure("cannot construct " + typeLabel(targetType)
                + ": no public constructor accepts " + args.length + " fields"
                + (lastConstructor == null ? "" : ""));
    }

    private static Object[] convertArgs(Class<?>[] parameterTypes, Object[] args) {
        var out = new Object[args.length];
        for (int index = 0; index < args.length; index++) {
            out[index] = convertArg(parameterTypes[index], args[index]);
        }
        return out;
    }

    private static Object convertArg(Class<?> parameterType, Object value) {
        if (value == null || parameterType.isInstance(value)) {
            return value;
        }
        if ((parameterType == Long.class || parameterType == Long.TYPE) && value instanceof Number number) {
            return number.longValue();
        }
        if ((parameterType == Integer.class || parameterType == Integer.TYPE) && value instanceof Number number) {
            return number.intValue();
        }
        if ((parameterType == Double.class || parameterType == Double.TYPE) && value instanceof Number number) {
            return number.doubleValue();
        }
        if ((parameterType == Float.class || parameterType == Float.TYPE) && value instanceof Number number) {
            return number.floatValue();
        }
        if ((parameterType == Boolean.class || parameterType == Boolean.TYPE) && value instanceof Boolean bool) {
            return bool;
        }
        if (parameterType == String.class) {
            return String.valueOf(value);
        }
        return value;
    }

    private static LumeType loadType(String simpleName, String contextPackage) {
        var className = simpleName.contains(".")
                ? simpleName
                : (contextPackage == null || contextPackage.isBlank() ? simpleName : contextPackage + "." + simpleName);
        try {
            Class<?> clazz = Class.forName(className);
            Field typeField = clazz.getField("TYPE");
            return (LumeType) typeField.get(null);
        } catch (ReflectiveOperationException err) {
            throw new DecodeFailure("cannot decode JSON as " + simpleName + ": type descriptor is not available");
        }
    }

    private static boolean isOptionDescriptor(String descriptor) {
        return descriptor.equals("Option") || descriptor.startsWith("Option[");
    }

    private static boolean isArrayDescriptor(String descriptor) {
        return descriptor.startsWith("[") || descriptor.startsWith("Array[");
    }

    private static Object defaultForDescriptor(String descriptor) {
        var trimmed = descriptor.trim();
        if (isArrayDescriptor(trimmed)) {
            return LumeArray.empty();
        }
        return switch (trimmed) {
            case "Str" -> "";
            case "Int", "Int64" -> 0L;
            case "Rune" -> 0;
            case "Float", "Float64" -> 0.0;
            case "Bool" -> false;
            default -> NO_DEFAULT;
        };
    }

    private static String arrayInnerDescriptor(String descriptor) {
        if (descriptor.startsWith("[") && descriptor.endsWith("]")) {
            return descriptor.substring(1, descriptor.length() - 1).trim();
        }
        return innerDescriptor(descriptor, "Array");
    }

    private static String innerDescriptor(String descriptor, String outer) {
        var prefix = outer + "[";
        if (descriptor.startsWith(prefix) && descriptor.endsWith("]")) {
            return descriptor.substring(prefix.length(), descriptor.length() - 1).trim();
        }
        return "Any";
    }

    private static Long toLong(Object value, String target) {
        if (value instanceof Number number) {
            return number.longValue();
        }
        if (value instanceof String text) {
            return Long.parseLong(text);
        }
        throw new DecodeFailure("expected " + target);
    }

    private static Double toDouble(Object value, String target) {
        if (value instanceof Number number) {
            return number.doubleValue();
        }
        if (value instanceof String text) {
            return Double.parseDouble(text);
        }
        throw new DecodeFailure("expected " + target);
    }

    private static Boolean toBool(Object value, String target) {
        if (value instanceof Boolean bool) {
            return bool;
        }
        if (value instanceof String text) {
            return Boolean.parseBoolean(text);
        }
        throw new DecodeFailure("expected " + target);
    }

    private static String typeName(LumeType type) {
        var name = type.name();
        return name.isDefined() ? (String) LumeRuntime.extractSuccessValue(name) : type.toString();
    }

    private static String typeLabel(LumeType type) {
        var qualified = type.qualifiedName();
        if (qualified.isDefined()) {
            return (String) LumeRuntime.extractSuccessValue(qualified);
        }
        return typeName(type);
    }

    private static String packageName(LumeType type) {
        var qualified = type.qualifiedName();
        if (!qualified.isDefined()) {
            return "";
        }
        var name = (String) LumeRuntime.extractSuccessValue(qualified);
        var dot = name.lastIndexOf('.');
        return dot < 0 ? "" : name.substring(0, dot);
    }

    private static String reflectionMessage(Throwable err) {
        var cause = err instanceof InvocationTargetException invocation && invocation.getCause() != null
                ? invocation.getCause()
                : err;
        return cause.getMessage() == null ? cause.getClass().getSimpleName() : cause.getMessage();
    }

    private static final class DecodeFailure extends RuntimeException {
        DecodeFailure(String message) {
            super(message);
        }
    }

    private static final class Parser {
        private final String text;
        private int index;

        Parser(String text) {
            this.text = text;
        }

        Object parse() {
            skipWhitespace();
            var value = parseValue();
            skipWhitespace();
            if (!isAtEnd()) {
                throw error("unexpected trailing content");
            }
            return value;
        }

        private Object parseValue() {
            skipWhitespace();
            if (isAtEnd()) {
                throw error("expected JSON value");
            }
            var ch = peek();
            if (ch == '"') {
                return parseString();
            }
            if (ch == '{') {
                return parseObject();
            }
            if (ch == '[') {
                return parseArray();
            }
            if (ch == 't') {
                expectLiteral("true");
                return Boolean.TRUE;
            }
            if (ch == 'f') {
                expectLiteral("false");
                return Boolean.FALSE;
            }
            if (ch == 'n') {
                expectLiteral("null");
                return null;
            }
            if (ch == '-' || Character.isDigit(ch)) {
                return parseNumber();
            }
            throw error("unexpected JSON token '" + ch + "'");
        }

        private Map<String, Object> parseObject() {
            consume('{');
            var out = new LinkedHashMap<String, Object>();
            skipWhitespace();
            if (tryConsume('}')) {
                return out;
            }
            while (true) {
                skipWhitespace();
                if (peek() != '"') {
                    throw error("expected object field name");
                }
                var key = parseString();
                skipWhitespace();
                consume(':');
                out.put(key, parseValue());
                skipWhitespace();
                if (tryConsume('}')) {
                    return out;
                }
                consume(',');
            }
        }

        private List<Object> parseArray() {
            consume('[');
            var out = new ArrayList<Object>();
            skipWhitespace();
            if (tryConsume(']')) {
                return out;
            }
            while (true) {
                out.add(parseValue());
                skipWhitespace();
                if (tryConsume(']')) {
                    return out;
                }
                consume(',');
            }
        }

        private String parseString() {
            consume('"');
            var out = new StringBuilder();
            while (!isAtEnd()) {
                var ch = advance();
                if (ch == '"') {
                    return out.toString();
                }
                if (ch == '\\') {
                    if (isAtEnd()) {
                        throw error("unterminated escape sequence");
                    }
                    var escaped = advance();
                    switch (escaped) {
                        case '"' -> out.append('"');
                        case '\\' -> out.append('\\');
                        case '/' -> out.append('/');
                        case 'b' -> out.append('\b');
                        case 'f' -> out.append('\f');
                        case 'n' -> out.append('\n');
                        case 'r' -> out.append('\r');
                        case 't' -> out.append('\t');
                        case 'u' -> out.append(parseUnicodeEscape());
                        default -> throw error("invalid escape sequence '\\" + escaped + "'");
                    }
                } else {
                    out.append(ch);
                }
            }
            throw error("unterminated string");
        }

        private char parseUnicodeEscape() {
            if (index + 4 > text.length()) {
                throw error("invalid unicode escape");
            }
            var hex = text.substring(index, index + 4);
            index += 4;
            try {
                return (char) Integer.parseInt(hex, 16);
            } catch (NumberFormatException err) {
                throw error("invalid unicode escape");
            }
        }

        private Number parseNumber() {
            var start = index;
            if (peek() == '-') {
                advance();
            }
            while (!isAtEnd() && Character.isDigit(peek())) {
                advance();
            }
            var floating = false;
            if (!isAtEnd() && peek() == '.') {
                floating = true;
                advance();
                while (!isAtEnd() && Character.isDigit(peek())) {
                    advance();
                }
            }
            if (!isAtEnd() && (peek() == 'e' || peek() == 'E')) {
                floating = true;
                advance();
                if (!isAtEnd() && (peek() == '+' || peek() == '-')) {
                    advance();
                }
                while (!isAtEnd() && Character.isDigit(peek())) {
                    advance();
                }
            }
            var raw = text.substring(start, index);
            try {
                return floating ? Double.parseDouble(raw) : Long.parseLong(raw);
            } catch (NumberFormatException err) {
                throw error("invalid number");
            }
        }

        private void expectLiteral(String literal) {
            if (!text.startsWith(literal, index)) {
                throw error("expected '" + literal + "'");
            }
            index += literal.length();
        }

        private void consume(char expected) {
            skipWhitespace();
            if (isAtEnd() || peek() != expected) {
                throw error("expected '" + expected + "'");
            }
            index++;
        }

        private boolean tryConsume(char expected) {
            skipWhitespace();
            if (!isAtEnd() && peek() == expected) {
                index++;
                return true;
            }
            return false;
        }

        private char advance() {
            return text.charAt(index++);
        }

        private char peek() {
            return text.charAt(index);
        }

        private boolean isAtEnd() {
            return index >= text.length();
        }

        private void skipWhitespace() {
            while (!isAtEnd() && Character.isWhitespace(peek())) {
                index++;
            }
        }

        private DecodeFailure error(String message) {
            return new DecodeFailure(message + " at offset " + index);
        }
    }
}
