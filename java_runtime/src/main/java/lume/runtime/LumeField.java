package lume.runtime;

public final class LumeField {
    private final String name;
    private final LumeType fieldType;

    private LumeField(String name, LumeType fieldType) {
        this.name = name;
        this.fieldType = fieldType;
    }

    public static LumeField of(String name, LumeType fieldType) {
        return new LumeField(name, fieldType);
    }

    public String name() {
        return name;
    }

    public LumeType fieldType() {
        return fieldType;
    }

    public LumeType runtimeType() {
        return LumeType.primitive("Field");
    }

    @Override
    public String toString() {
        return name;
    }
}
