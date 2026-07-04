package lume.db;

import java.lang.reflect.Constructor;
import java.lang.reflect.InvocationTargetException;
import java.util.List;

import lume.core.LumeField;
import lume.core.LumeType;
import lume.core.LumeTypeKind;
import lume.core.Result;

public final class JdbcRowDecoder {
    private final LumeType targetType;
    private final List<LumeField> fields;
    private final ConstructorPlan[] constructors;

    private JdbcRowDecoder(
            LumeType targetType,
            List<LumeField> fields,
            ConstructorPlan[] constructors) {
        this.targetType = targetType;
        this.fields = List.copyOf(fields);
        this.constructors = constructors.clone();
    }

    public static Result<JdbcRowDecoder, DbError> prepare(LumeType targetType) {
        try {
            return Jdbc.ok(create(targetType));
        } catch (JdbcRow.DecodeFailure err) {
            return Jdbc.err(err.getMessage());
        }
    }

    public <T> Result<T, DbError> decode(JdbcRow row) {
        try {
            @SuppressWarnings("unchecked")
            var decoded = (T) instantiate(row);
            return Jdbc.ok(decoded);
        } catch (JdbcRow.DecodeFailure err) {
            return Jdbc.err(err.getMessage());
        }
    }

    private static JdbcRowDecoder create(LumeType targetType) throws JdbcRow.DecodeFailure {
        var kind = targetType.kind();
        if (kind != LumeTypeKind.Class && kind != LumeTypeKind.Shape) {
            throw new JdbcRow.DecodeFailure("cannot decode SQL row as " + JdbcRow.typeLabel(targetType)
                    + ": only class and shape descriptors are supported");
        }

        var qualifiedName = targetType.qualifiedName();
        if (!qualifiedName.isDefined()) {
            throw new JdbcRow.DecodeFailure("cannot decode SQL row as " + JdbcRow.typeLabel(targetType)
                    + ": type has no Java qualified name");
        }

        Class<?> targetClass;
        try {
            targetClass = Class.forName(qualifiedName.orPanic());
        } catch (ClassNotFoundException err) {
            throw new JdbcRow.DecodeFailure("cannot decode SQL row as " + JdbcRow.typeLabel(targetType)
                    + ": Java class '" + qualifiedName.orPanic() + "' is not available");
        }

        var fields = targetType.fields().asJava().stream().toList();
        var constructors = java.util.Arrays.stream(targetClass.getConstructors())
                .filter(constructor -> constructor.getParameterCount() == fields.size())
                .map(ConstructorPlan::new)
                .toArray(ConstructorPlan[]::new);

        if (constructors.length == 0) {
            throw new JdbcRow.DecodeFailure("cannot decode SQL row as " + JdbcRow.typeLabel(targetType)
                    + ": no public constructor accepts " + fields.size() + " fields");
        }

        return new JdbcRowDecoder(targetType, fields, constructors);
    }

    private Object instantiate(JdbcRow row) throws JdbcRow.DecodeFailure {
        JdbcRow.DecodeFailure lastFailure = null;
        for (var constructor : constructors) {
            try {
                return constructor.invoke(row, fields);
            } catch (JdbcRow.DecodeFailure err) {
                lastFailure = err;
            }
        }

        if (lastFailure != null) {
            throw lastFailure;
        }
        throw new JdbcRow.DecodeFailure("cannot decode SQL row as " + JdbcRow.typeLabel(targetType)
                + ": no public constructor accepts " + fields.size() + " fields");
    }

    private static final class ConstructorPlan {
        private final Constructor<?> constructor;
        private final Class<?>[] parameterTypes;

        ConstructorPlan(Constructor<?> constructor) {
            this.constructor = constructor;
            this.parameterTypes = constructor.getParameterTypes();
        }

        Object invoke(JdbcRow row, List<LumeField> fields) throws JdbcRow.DecodeFailure {
            var args = new Object[fields.size()];
            for (int index = 0; index < fields.size(); index++) {
                args[index] = row.decodeField(fields.get(index), parameterTypes[index]);
            }
            try {
                return constructor.newInstance(args);
            } catch (InstantiationException | IllegalAccessException err) {
                throw new JdbcRow.DecodeFailure("cannot construct " + constructor.getDeclaringClass().getName()
                        + ": " + err.getMessage());
            } catch (InvocationTargetException err) {
                var cause = err.getCause() == null ? err : err.getCause();
                throw new JdbcRow.DecodeFailure("cannot construct " + constructor.getDeclaringClass().getName()
                        + ": " + cause.getMessage());
            }
        }
    }
}
