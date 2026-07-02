package lume.core;

import java.util.LinkedHashMap;
import java.util.Map;

public final class LumeAnnotation {
    private final String name;
    private final LinkedHashMap<String, Object> fields;

    private LumeAnnotation(String name, LinkedHashMap<String, Object> fields) {
        this.name = name;
        this.fields = new LinkedHashMap<>(fields);
    }

    public static LumeAnnotation of(String name, LumeAnnotationField[] fields) {
        var map = new LinkedHashMap<String, Object>();
        for (var field : fields) {
            map.put(field.name(), field.value());
        }
        return new LumeAnnotation(name, map);
    }

    public String name() {
        return name;
    }

    public Option<Object> field(String name) {
        return fields.containsKey(name) ? Option.some(fields.get(name)) : Option.none();
    }

    public Option<String> str(String name) {
        Option<Object> value = field(name);
        return value.isDefined() ? Option.some(String.valueOf(value.orPanic())) : Option.none();
    }

    public Map<String, Object> asJava() {
        return Map.copyOf(fields);
    }

    public LumeType runtimeType() {
        return LumeType.primitive("AnnotationValue");
    }

    @Override
    public String toString() {
        return "@" + name + fields;
    }
}
