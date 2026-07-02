package lume.core;

public final class LumeParam {
    private final String name;
    private final LumeType paramType;

    private LumeParam(String name, LumeType paramType) {
        this.name = name;
        this.paramType = paramType;
    }

    public static LumeParam of(String name, LumeType paramType) {
        return new LumeParam(name, paramType);
    }

    public String name() {
        return name;
    }

    public LumeType paramType() {
        return paramType;
    }

    public LumeType runtimeType() {
        return LumeType.primitive("Param");
    }

    @Override
    public String toString() {
        return name;
    }
}
