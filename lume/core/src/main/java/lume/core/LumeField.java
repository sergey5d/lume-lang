package lume.core;

public final class LumeField {
    private final String name;
    private final LumeType fieldType;
    private final java.util.List<LumeAnnotation> annotations;

    private LumeField(String name, LumeType fieldType) {
        this(name, fieldType, java.util.List.of());
    }

    private LumeField(String name, LumeType fieldType, java.util.List<LumeAnnotation> annotations) {
        this.name = name;
        this.fieldType = fieldType;
        this.annotations = java.util.List.copyOf(annotations);
    }

    public static LumeField of(String name, LumeType fieldType) {
        return new LumeField(name, fieldType);
    }

    public static LumeField of(String name, LumeType fieldType, LumeAnnotation[] annotations) {
        return new LumeField(name, fieldType, java.util.List.of(annotations));
    }

    public String name() {
        return name;
    }

    public LumeType fieldType() {
        return fieldType;
    }

    public Option<LumeAnnotation> annotation(LumeType annotationType) {
        return annotation(annotationName(annotationType));
    }

    public Boolean hasAnnotation(LumeType annotationType) {
        return annotation(annotationType).isDefined();
    }

    public Option<LumeAnnotation> annotation(String name) {
        return annotations.stream()
                .filter(annotation -> annotation.name().equals(name))
                .findFirst()
                .map(LumeRuntime::optionSome)
                .orElseGet(LumeRuntime::optionNone);
    }

    public Boolean hasAnnotation(String name) {
        return annotation(name).isDefined();
    }

    private static String annotationName(LumeType annotationType) {
        return annotationType.name().orPanic();
    }

    public LumeType runtimeType() {
        return LumeType.primitive("Field");
    }

    @Override
    public String toString() {
        return name;
    }
}
