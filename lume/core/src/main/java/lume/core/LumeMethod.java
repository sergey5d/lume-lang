package lume.core;

import java.util.List;
import java.util.function.BiFunction;

public final class LumeMethod {
    private final String name;
    private final List<LumeParam> params;
    private final LumeType returnType;
    private final List<LumeAnnotation> annotations;
    private final BiFunction<Object, Object[], Object> invoker;

    private LumeMethod(
            String name,
            List<LumeParam> params,
            LumeType returnType,
            List<LumeAnnotation> annotations,
            BiFunction<Object, Object[], Object> invoker) {
        this.name = name;
        this.params = List.copyOf(params);
        this.returnType = returnType;
        this.annotations = List.copyOf(annotations);
        this.invoker = invoker;
    }

    public static LumeMethod of(String name, LumeType returnType, LumeParam[] params) {
        return new LumeMethod(name, List.of(params), returnType, List.of(), null);
    }

    public static LumeMethod of(
            String name,
            LumeType returnType,
            LumeParam[] params,
            LumeAnnotation[] annotations,
            BiFunction<Object, Object[], Object> invoker) {
        return new LumeMethod(name, List.of(params), returnType, List.of(annotations), invoker);
    }

    public String name() {
        return name;
    }

    public LumeArray<LumeParam> params() {
        return LumeArray.from(params);
    }

    public LumeType returnType() {
        return returnType;
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

    public Object invoke(Object receiver, Object... args) {
        if (invoker == null) {
            throw new LumePanic("method '" + name + "' is not invokable");
        }
        return invoker.apply(receiver, args);
    }

    public Result<Object, ReflectionError> call(Object receiver, Object... args) {
        try {
            return new Result.Ok<>(invoke(receiver, args));
        } catch (Throwable err) {
            return new Result.Err<>(new ReflectionError(reflectionMessage(err)));
        }
    }

    private static String reflectionMessage(Throwable err) {
        if (err instanceof LumePanic panic) {
            return panic.getMessage();
        }
        return err.getMessage() == null ? err.getClass().getSimpleName() : err.getMessage();
    }

    public LumeType runtimeType() {
        return LumeType.primitive("Method");
    }

    @Override
    public String toString() {
        return name;
    }
}
