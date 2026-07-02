package lume.core;

import java.util.List;

public final class LumeEnumCase {
    private final String name;
    private final List<LumeField> fields;
    private final List<LumeAnnotation> annotations;

    private LumeEnumCase(String name, List<LumeField> fields) {
        this(name, fields, List.of());
    }

    private LumeEnumCase(String name, List<LumeField> fields, List<LumeAnnotation> annotations) {
        this.name = name;
        this.fields = List.copyOf(fields);
        this.annotations = List.copyOf(annotations);
    }

    public static LumeEnumCase of(String name, LumeField[] fields) {
        return new LumeEnumCase(name, List.of(fields));
    }

    public static LumeEnumCase of(String name, LumeField[] fields, LumeAnnotation[] annotations) {
        return new LumeEnumCase(name, List.of(fields), List.of(annotations));
    }

    public String name() {
        return name;
    }

    public LumeList<LumeField> fields() {
        return LumeList.from(fields);
    }

    public Option<LumeAnnotation> annotation(String name) {
        return annotations.stream()
                .filter(annotation -> annotation.name().equals(name))
                .findFirst()
                .map(Option::some)
                .orElseGet(Option::none);
    }

    public Boolean hasAnnotation(String name) {
        return annotation(name).isDefined();
    }

    public LumeType runtimeType() {
        return LumeType.primitive("EnumCase");
    }

    @Override
    public String toString() {
        return name;
    }
}
