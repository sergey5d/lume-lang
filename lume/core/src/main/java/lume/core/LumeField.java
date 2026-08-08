package lume.core;

public final class LumeField {
    private final String name;
    private final LumeType fieldType;
    private final java.util.List<LumeAnnotation> annotations;
    private final boolean hidden;

    private LumeField(String name, LumeType fieldType) {
        this(name, fieldType, java.util.List.of(), false);
    }

    private LumeField(String name, LumeType fieldType, java.util.List<LumeAnnotation> annotations) {
        this(name, fieldType, annotations, false);
    }

    private LumeField(String name, LumeType fieldType, java.util.List<LumeAnnotation> annotations, boolean hidden) {
        this.name = name;
        this.fieldType = fieldType;
        this.annotations = java.util.List.copyOf(annotations);
        this.hidden = hidden;
    }

    public static LumeField of(String name, LumeType fieldType) {
        return new LumeField(name, fieldType);
    }

    public static LumeField of(String name, LumeType fieldType, LumeAnnotation[] annotations) {
        return new LumeField(name, fieldType, java.util.List.of(annotations));
    }

    public static LumeField of(String name, LumeType fieldType, LumeAnnotation[] annotations, boolean hidden) {
        return new LumeField(name, fieldType, java.util.List.of(annotations), hidden);
    }

    public String name() {
        return name;
    }

    public LumeType fieldType() {
        return fieldType;
    }

    public Boolean hidden() {
        return hidden;
    }

    public Boolean isHidden() {
        return hidden;
    }

    public Result<Object, ReflectionError> get(Object receiver) {
        if (receiver == null) {
            return new Result.Err<>(new ReflectionError("cannot read field '" + name + "' from null"));
        }

        try {
            var field = receiver.getClass().getDeclaredField(name);
            field.setAccessible(true);
            return new Result.Ok<>(field.get(receiver));
        } catch (NoSuchFieldException missingField) {
            try {
                var accessor = receiver.getClass().getDeclaredMethod(name);
                accessor.setAccessible(true);
                return new Result.Ok<>(accessor.invoke(receiver));
            } catch (ReflectiveOperationException err) {
                return new Result.Err<>(new ReflectionError(
                        "cannot read field '" + name + "' from " + receiver.getClass().getName() + ": "
                                + reflectionMessage(err)));
            }
        } catch (ReflectiveOperationException err) {
            return new Result.Err<>(new ReflectionError(
                    "cannot read field '" + name + "' from " + receiver.getClass().getName() + ": "
                            + reflectionMessage(err)));
        }
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
        return (String) LumeRuntime.extractSuccessValue(annotationType.name());
    }

    private static String reflectionMessage(Throwable err) {
        var cause = err instanceof java.lang.reflect.InvocationTargetException invocation
                && invocation.getCause() != null
                ? invocation.getCause()
                : err;
        return cause.getMessage() == null ? cause.getClass().getSimpleName() : cause.getMessage();
    }

    public LumeType runtimeType() {
        return LumeType.primitive("Field");
    }

    @Override
    public String toString() {
        return name;
    }
}
