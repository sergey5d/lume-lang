package lume.core;

import java.lang.reflect.InvocationTargetException;
import java.util.List;

public final class LumeEnumCase {
    private final String ownerQualifiedName;
    private final String name;
    private final List<LumeField> fields;
    private final List<LumeAnnotation> annotations;

    private LumeEnumCase(String name, List<LumeField> fields) {
        this(null, name, fields, List.of());
    }

    private LumeEnumCase(String name, List<LumeField> fields, List<LumeAnnotation> annotations) {
        this(null, name, fields, annotations);
    }

    private LumeEnumCase(
            String ownerQualifiedName,
            String name,
            List<LumeField> fields,
            List<LumeAnnotation> annotations) {
        this.ownerQualifiedName = ownerQualifiedName;
        this.name = name;
        this.fields = List.copyOf(fields);
        this.annotations = List.copyOf(annotations);
    }

    public static LumeEnumCase of(String name, LumeField[] fields) {
        return new LumeEnumCase(name, List.of(fields));
    }

    public static LumeEnumCase of(String name, LumeField[] fields, LumeAnnotation[] annotations) {
        return new LumeEnumCase(name, List.of(fields), List.of(annotations));
    }

    public static LumeEnumCase of(
            String ownerQualifiedName,
            String name,
            LumeField[] fields,
            LumeAnnotation[] annotations) {
        return new LumeEnumCase(ownerQualifiedName, name, List.of(fields), List.of(annotations));
    }

    public String name() {
        return name;
    }

    public LumeVector<LumeField> fields() {
        return LumeVector.from(fields);
    }

    public Result<Object, ReflectionError> construct(Object... args) {
        if (ownerQualifiedName == null || ownerQualifiedName.isBlank()) {
            return reflectionErr("cannot construct enum case '" + name + "': parent enum type is unknown");
        }

        Class<?> caseClass;
        try {
            caseClass = Class.forName(ownerQualifiedName + "$" + name);
        } catch (ClassNotFoundException err) {
            return reflectionErr("cannot construct enum case '" + name + "': Java case class '"
                    + ownerQualifiedName + "$" + name + "' is not available");
        }

        ReflectionError lastError = null;
        for (var constructor : caseClass.getConstructors()) {
            if (constructor.getParameterCount() != args.length) {
                continue;
            }
            try {
                return new Result.Ok<>(constructor.newInstance(convertArgs(constructor.getParameterTypes(), args)));
            } catch (ReflectiveOperationException | RuntimeException err) {
                lastError = new ReflectionError(
                        "cannot construct enum case '" + name + "': " + reflectionMessage(err));
            }
        }

        if (lastError != null) {
            return new Result.Err<>(lastError);
        }
        return reflectionErr("cannot construct enum case '" + name + "': no public constructor accepts "
                + args.length + " arguments");
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

    private static Result<Object, ReflectionError> reflectionErr(String message) {
        return new Result.Err<>(new ReflectionError(message));
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
        return LumeType.primitive("EnumCase");
    }

    @Override
    public String toString() {
        return name;
    }
}
