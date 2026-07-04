package lume.core;

import java.lang.reflect.InvocationTargetException;
import java.util.List;

public final class LumeType {
    private final String name;
    private final String qualifiedName;
    private final LumeTypeKind kind;
    private final List<LumeField> fields;
    private final List<LumeMethod> methods;
    private final List<LumeEnumCase> cases;
    private final List<LumeAnnotation> annotations;

    private LumeType(
            String name,
            String qualifiedName,
            LumeTypeKind kind,
            List<LumeField> fields,
            List<LumeMethod> methods,
            List<LumeEnumCase> cases,
            List<LumeAnnotation> annotations) {
        this.name = name;
        this.qualifiedName = qualifiedName;
        this.kind = kind;
        this.fields = List.copyOf(fields);
        this.methods = List.copyOf(methods);
        this.cases = List.copyOf(cases);
        this.annotations = List.copyOf(annotations);
    }

    public static LumeType primitive(String name) {
        return new LumeType(name, name, LumeTypeKind.Primitive, List.of(), List.of(), List.of(), List.of());
    }

    public static LumeType classType(
            String name,
            String qualifiedName,
            LumeField[] fields,
            LumeMethod[] methods) {
        return classType(name, qualifiedName, fields, methods, new LumeAnnotation[] {});
    }

    public static LumeType classType(
            String name,
            String qualifiedName,
            LumeField[] fields,
            LumeMethod[] methods,
            LumeAnnotation[] annotations) {
        return new LumeType(
                name,
                qualifiedName,
                LumeTypeKind.Class,
                List.of(fields),
                List.of(methods),
                List.of(),
                List.of(annotations));
    }

    public static LumeType shapeType(
            String name,
            String qualifiedName,
            LumeField[] fields,
            LumeMethod[] methods) {
        return shapeType(name, qualifiedName, fields, methods, new LumeAnnotation[] {});
    }

    public static LumeType shapeType(
            String name,
            String qualifiedName,
            LumeField[] fields,
            LumeMethod[] methods,
            LumeAnnotation[] annotations) {
        return new LumeType(
                name,
                qualifiedName,
                LumeTypeKind.Shape,
                List.of(fields),
                List.of(methods),
                List.of(),
                List.of(annotations));
    }

    public static LumeType enumType(
            String name,
            String qualifiedName,
            LumeEnumCase[] cases,
            LumeMethod[] methods) {
        return enumType(name, qualifiedName, cases, methods, new LumeAnnotation[] {});
    }

    public static LumeType enumType(
            String name,
            String qualifiedName,
            LumeEnumCase[] cases,
            LumeMethod[] methods,
            LumeAnnotation[] annotations) {
        return new LumeType(
                name,
                qualifiedName,
                LumeTypeKind.Enum,
                List.of(),
                List.of(methods),
                List.of(cases),
                List.of(annotations));
    }

    public static LumeType interfaceType(String name, String qualifiedName, LumeMethod[] methods) {
        return interfaceType(name, qualifiedName, methods, new LumeAnnotation[] {});
    }

    public static LumeType interfaceType(
            String name,
            String qualifiedName,
            LumeMethod[] methods,
            LumeAnnotation[] annotations) {
        return new LumeType(
                name,
                qualifiedName,
                LumeTypeKind.Interface,
                List.of(),
                List.of(methods),
                List.of(),
                List.of(annotations));
    }

    public static LumeType singleType(
            String name,
            String qualifiedName,
            LumeField[] fields,
            LumeMethod[] methods) {
        return singleType(name, qualifiedName, fields, methods, new LumeAnnotation[] {});
    }

    public static LumeType singleType(
            String name,
            String qualifiedName,
            LumeField[] fields,
            LumeMethod[] methods,
            LumeAnnotation[] annotations) {
        return new LumeType(
                name,
                qualifiedName,
                LumeTypeKind.Single,
                List.of(fields),
                List.of(methods),
                List.of(),
                List.of(annotations));
    }

    public static LumeType annotationType(String name, String qualifiedName, LumeField[] fields) {
        return annotationType(name, qualifiedName, fields, new LumeAnnotation[] {});
    }

    public static LumeType annotationType(
            String name,
            String qualifiedName,
            LumeField[] fields,
            LumeAnnotation[] annotations) {
        return new LumeType(
                name,
                qualifiedName,
                LumeTypeKind.Annotation,
                List.of(fields),
                List.of(),
                List.of(),
                List.of(annotations));
    }

    public Option<String> name() {
        return name == null || name.isEmpty() ? LumeRuntime.optionNone() : LumeRuntime.optionSome(name);
    }

    public Option<String> qualifiedName() {
        return qualifiedName == null || qualifiedName.isEmpty()
                ? LumeRuntime.optionNone()
                : LumeRuntime.optionSome(qualifiedName);
    }

    public LumeTypeKind kind() {
        return kind;
    }

    public Option<LumeType> asClass() {
        return kind == LumeTypeKind.Class ? LumeRuntime.optionSome(this) : LumeRuntime.optionNone();
    }

    public Option<LumeType> asShape() {
        return kind == LumeTypeKind.Shape ? LumeRuntime.optionSome(this) : LumeRuntime.optionNone();
    }

    public Option<LumeType> asEnum() {
        return kind == LumeTypeKind.Enum ? LumeRuntime.optionSome(this) : LumeRuntime.optionNone();
    }

    public Option<LumeType> asInterface() {
        return kind == LumeTypeKind.Interface ? LumeRuntime.optionSome(this) : LumeRuntime.optionNone();
    }

    public Option<LumeType> asSingle() {
        return kind == LumeTypeKind.Single ? LumeRuntime.optionSome(this) : LumeRuntime.optionNone();
    }

    public Option<LumeType> asAnnotation() {
        return kind == LumeTypeKind.Annotation ? LumeRuntime.optionSome(this) : LumeRuntime.optionNone();
    }

    public LumeList<LumeField> fields() {
        return LumeList.from(fields);
    }

    public LumeList<LumeMethod> methods() {
        return LumeList.from(methods);
    }

    public Option<LumeField> field(String name) {
        return fields.stream()
                .filter(field -> field.name().equals(name))
                .findFirst()
                .map(LumeRuntime::optionSome)
                .orElseGet(LumeRuntime::optionNone);
    }

    public Option<LumeMethod> method(String name) {
        return methods.stream()
                .filter(method -> method.name().equals(name))
                .findFirst()
                .map(LumeRuntime::optionSome)
                .orElseGet(LumeRuntime::optionNone);
    }

    public <T> Result<T, ReflectionError> construct(Object... args) {
        if (kind != LumeTypeKind.Class && kind != LumeTypeKind.Shape) {
            return reflectionErr("cannot construct " + typeLabel()
                    + ": only class and shape descriptors are constructable");
        }
        if (qualifiedName == null || qualifiedName.isBlank()) {
            return reflectionErr("cannot construct " + typeLabel() + ": type has no Java qualified name");
        }

        Class<?> targetClass;
        try {
            targetClass = Class.forName(qualifiedName);
        } catch (ClassNotFoundException err) {
            return reflectionErr("cannot construct " + typeLabel() + ": Java class '" + qualifiedName
                    + "' is not available");
        }

        ReflectionError lastError = null;
        for (var constructor : targetClass.getConstructors()) {
            if (constructor.getParameterCount() != args.length) {
                continue;
            }
            try {
                @SuppressWarnings("unchecked")
                var value = (T) constructor.newInstance(convertArgs(constructor.getParameterTypes(), args));
                return new Result.Ok<>(value);
            } catch (ReflectiveOperationException | RuntimeException err) {
                lastError = new ReflectionError("cannot construct " + typeLabel() + ": " + reflectionMessage(err));
            }
        }

        if (lastError != null) {
            return new Result.Err<>(lastError);
        }
        return reflectionErr("cannot construct " + typeLabel() + ": no public constructor accepts "
                + args.length + " arguments");
    }

    public LumeList<LumeEnumCase> cases() {
        return LumeList.from(cases);
    }

    public Option<LumeEnumCase> case_(String name) {
        return cases.stream()
                .filter(enumCase -> enumCase.name().equals(name))
                .findFirst()
                .map(LumeRuntime::optionSome)
                .orElseGet(LumeRuntime::optionNone);
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

    private <T> Result<T, ReflectionError> reflectionErr(String message) {
        return new Result.Err<>(new ReflectionError(message));
    }

    private String typeLabel() {
        if (qualifiedName != null && !qualifiedName.isBlank()) {
            return qualifiedName;
        }
        if (name != null && !name.isBlank()) {
            return name;
        }
        return kind.toString();
    }

    private static Object[] convertArgs(Class<?>[] parameterTypes, Object[] args) {
        var out = new Object[args.length];
        for (int index = 0; index < args.length; index++) {
            out[index] = convertArg(parameterTypes[index], args[index]);
        }
        return out;
    }

    private static Object convertArg(Class<?> parameterType, Object value) {
        if (value == null || parameterType.isInstance(value)) {
            return value;
        }
        if ((parameterType == Long.class || parameterType == Long.TYPE) && value instanceof Number number) {
            return number.longValue();
        }
        if ((parameterType == Integer.class || parameterType == Integer.TYPE) && value instanceof Number number) {
            return number.intValue();
        }
        if ((parameterType == Double.class || parameterType == Double.TYPE) && value instanceof Number number) {
            return number.doubleValue();
        }
        if ((parameterType == Float.class || parameterType == Float.TYPE) && value instanceof Number number) {
            return number.floatValue();
        }
        if (parameterType == String.class) {
            return String.valueOf(value);
        }
        return value;
    }

    private static String reflectionMessage(Throwable err) {
        var cause = err instanceof InvocationTargetException invocation && invocation.getCause() != null
                ? invocation.getCause()
                : err;
        return cause.getMessage() == null ? cause.getClass().getSimpleName() : cause.getMessage();
    }

    public LumeType runtimeType() {
        return primitive("Type");
    }

    @Override
    public String toString() {
        return name == null || name.isEmpty() ? kind.toString() : name;
    }
}
