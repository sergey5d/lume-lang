package lume.runtime;

import java.util.List;

public final class LumeMethod {
    private final String name;
    private final List<LumeParam> params;
    private final LumeType returnType;
    private final List<LumeAnnotation> annotations;
    private final LumeMethodInvoker invoker;

    private LumeMethod(
            String name,
            List<LumeParam> params,
            LumeType returnType,
            List<LumeAnnotation> annotations,
            LumeMethodInvoker invoker) {
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
            LumeMethodInvoker invoker) {
        return new LumeMethod(name, List.of(params), returnType, List.of(annotations), invoker);
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

    public Object invoke(Object receiver, Object... args) {
        if (invoker == null) {
            throw new LumePanic("method '" + name + "' is not invokable");
        }
        return invoker.invoke(receiver, args);
    }

    public LumeType runtimeType() {
        return LumeType.primitive("Method");
    }

    @Override
    public String toString() {
        return name;
    }
}
