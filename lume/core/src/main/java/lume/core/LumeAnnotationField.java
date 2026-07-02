package lume.core;

public final class LumeAnnotationField {
    private final String name;
    private final Object value;

    private LumeAnnotationField(String name, Object value) {
        this.name = name;
        this.value = value;
    }

    public static LumeAnnotationField of(String name, Object value) {
        return new LumeAnnotationField(name, value);
    }

    public String name() {
        return name;
    }

    public Object value() {
        return value;
    }
}
