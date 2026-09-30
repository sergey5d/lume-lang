package lume.core;

import java.util.Collections;
import java.util.LinkedHashMap;
import java.util.Map;

/** Runtime representation for an anonymous structural shape. */
public final class LumeShape {
    private final Map<String, Object> fields;

    private LumeShape(Map<String, Object> fields) {
        this.fields = Collections.unmodifiableMap(new LinkedHashMap<>(fields));
    }

    public static LumeShape of(Object... parts) {
        if (parts.length % 2 != 0) {
            throw new IllegalArgumentException("shape construction expects field-name/value pairs");
        }

        var fields = new LinkedHashMap<String, Object>();
        for (var index = 0; index < parts.length; index += 2) {
            if (!(parts[index] instanceof String name)) {
                throw new IllegalArgumentException("shape field names must be strings");
            }
            fields.put(name, parts[index + 1]);
        }
        return new LumeShape(fields);
    }

    public Object get(String name) {
        if (!fields.containsKey(name)) {
            throw new IllegalArgumentException("shape has no field '" + name + "'");
        }
        return fields.get(name);
    }

    @Override
    public boolean equals(Object other) {
        return other instanceof LumeShape shape && fields.equals(shape.fields);
    }

    @Override
    public int hashCode() {
        return fields.hashCode();
    }

    @Override
    public String toString() {
        return fields.toString();
    }
}
