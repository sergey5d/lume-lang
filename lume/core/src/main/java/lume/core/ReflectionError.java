package lume.core;

public final class ReflectionError {
    private final String message;

    public ReflectionError(String message) {
        this.message = message == null ? "reflection error" : message;
    }

    public String message() {
        return message;
    }

    public LumeType runtimeType() {
        return LumeType.classType(
                "ReflectionError",
                "lume.core.ReflectionError",
                new LumeField[] {LumeField.of("message", LumeType.primitive("Str"))},
                new LumeMethod[] {});
    }

    @Override
    public String toString() {
        return message;
    }
}
