package lume.runtime;

import java.util.List;

public final class LumeMethod {
    private final String name;
    private final List<LumeParam> params;
    private final LumeType returnType;

    private LumeMethod(String name, List<LumeParam> params, LumeType returnType) {
        this.name = name;
        this.params = List.copyOf(params);
        this.returnType = returnType;
    }

    public static LumeMethod of(String name, LumeType returnType, LumeParam[] params) {
        return new LumeMethod(name, List.of(params), returnType);
    }

    public String name() {
        return name;
    }

    public LumeList<LumeParam> params() {
        return LumeList.from(params);
    }

    public LumeType returnType() {
        return returnType;
    }

    public LumeType runtimeType() {
        return LumeType.primitive("Method");
    }

    @Override
    public String toString() {
        return name;
    }
}
