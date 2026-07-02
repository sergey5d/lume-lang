package lume.runtime;

import java.util.List;

public final class LumeEnumCase {
    private final String name;
    private final List<LumeField> fields;

    private LumeEnumCase(String name, List<LumeField> fields) {
        this.name = name;
        this.fields = List.copyOf(fields);
    }

    public static LumeEnumCase of(String name, LumeField[] fields) {
        return new LumeEnumCase(name, List.of(fields));
    }

    public String name() {
        return name;
    }

    public LumeList<LumeField> fields() {
        return LumeList.from(fields);
    }

    public LumeType runtimeType() {
        return LumeType.primitive("EnumCase");
    }

    @Override
    public String toString() {
        return name;
    }
}
