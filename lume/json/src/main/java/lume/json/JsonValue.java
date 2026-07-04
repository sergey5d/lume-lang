package lume.json;

import java.util.List;

public sealed interface JsonValue
        permits JsonValue.JsonNull, JsonValue.JsonBool, JsonValue.JsonNumber, JsonValue.JsonString,
        JsonValue.JsonArray, JsonValue.JsonObject {
    String stringify();

    record JsonNull() implements JsonValue {
        @Override
        public String stringify() {
            return "null";
        }
    }

    record JsonBool(Boolean value) implements JsonValue {
        @Override
        public String stringify() {
            return Boolean.TRUE.equals(value) ? "true" : "false";
        }
    }

    record JsonNumber(String value) implements JsonValue {
        @Override
        public String stringify() {
            return value;
        }
    }

    record JsonString(String value) implements JsonValue {
        @Override
        public String stringify() {
            return "\"" + JsonRuntime.escape(value) + "\"";
        }
    }

    record JsonArray(List<JsonValue> values) implements JsonValue {
        @Override
        public String stringify() {
            var out = new StringBuilder();
            out.append("[");
            for (int index = 0; index < values.size(); index++) {
                if (index > 0) {
                    out.append(",");
                }
                out.append(values.get(index).stringify());
            }
            out.append("]");
            return out.toString();
        }
    }

    record JsonObject(List<JsonField> fields) implements JsonValue {
        @Override
        public String stringify() {
            var out = new StringBuilder();
            out.append("{");
            for (int index = 0; index < fields.size(); index++) {
                if (index > 0) {
                    out.append(",");
                }
                var field = fields.get(index);
                out.append("\"").append(JsonRuntime.escape(field.name())).append("\":");
                out.append(field.value().stringify());
            }
            out.append("}");
            return out.toString();
        }
    }
}
