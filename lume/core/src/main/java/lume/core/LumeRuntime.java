package lume.core;

import java.lang.reflect.InvocationTargetException;
import java.util.Arrays;
import java.util.function.Supplier;
import java.util.stream.Collectors;

public final class LumeRuntime {
    private LumeRuntime() {
    }

    public static LumeUnit print(Object... values) {
        System.out.print(join(values));
        return LumeUnit.INSTANCE;
    }

    public static LumeUnit println(Object... values) {
        System.out.println(join(values));
        return LumeUnit.INSTANCE;
    }

    public static LumeUnit printf(String format, Object... values) {
        System.out.printf(format, values);
        return LumeUnit.INSTANCE;
    }

    public static LumeUnit assertTrue(Boolean condition, String message) {
        if (!Boolean.TRUE.equals(condition)) {
            throw new LumePanic(message);
        }
        return LumeUnit.INSTANCE;
    }

    public static <E> Result<LumeUnit, E> ensure(Boolean condition, Supplier<E> error) {
        if (Boolean.TRUE.equals(condition)) {
            return new Result.Ok<>(LumeUnit.INSTANCE);
        }
        return new Result.Err<>(error.get());
    }

    public static <T> Option<T> optionSome(T value) {
        return new Option.Some<>(value);
    }

    public static <T> Option<T> optionNone() {
        return new Option.None<>();
    }

    public static Option<Long> parseInt(String text) {
        try {
            return optionSome(Long.parseLong(text));
        } catch (NumberFormatException err) {
            return optionNone();
        }
    }

    public static Option<Double> parseFloat(String text) {
        try {
            return optionSome(Double.parseDouble(text));
        } catch (NumberFormatException err) {
            return optionNone();
        }
    }

    public static Boolean extractSuccessIsSet(Object value) {
        if (value instanceof Option<?> option) {
            return option.isDefined();
        }
        if (value instanceof Result<?, ?> result) {
            return result.isOk();
        }
        if (value instanceof Either<?, ?> either) {
            return either.isRight();
        }
        return false;
    }

    public static Boolean variantIs(Object value, String caseName) {
        return value != null && value.getClass().getSimpleName().equals(caseName);
    }

    public static Object variantField(Object value, String fieldName) {
        if (value == null) {
            throw new LumePanic("cannot read enum field '" + fieldName + "' from null");
        }

        try {
            var method = value.getClass().getMethod(fieldName);
            return method.invoke(value);
        } catch (NoSuchMethodException err) {
            // Pattern matching may probe fields before the case guard proves that this case matched.
            return null;
        } catch (IllegalAccessException | InvocationTargetException err) {
            throw new LumePanic("failed to read enum field '" + fieldName + "': " + err.getMessage());
        }
    }

    public static LumeType runtimeTypeOf(Object value) {
        if (value == null) {
            return LumeType.primitive("Null");
        }
        if (value instanceof String) {
            return LumeType.primitive("Str");
        }
        if (value instanceof Boolean) {
            return LumeType.primitive("Bool");
        }
        if (value instanceof Long || value instanceof Integer || value instanceof Short || value instanceof Byte) {
            return LumeType.primitive("Int");
        }
        if (value instanceof Double || value instanceof Float) {
            return LumeType.primitive("Float");
        }
        if (value instanceof LumeUnit) {
            return LumeType.primitive("Unit");
        }

        try {
            var method = value.getClass().getDeclaredMethod("runtimeType");
            method.setAccessible(true);
            Object result = method.invoke(value);
            if (result instanceof LumeType type) {
                return type;
            }
        } catch (NoSuchMethodException ignored) {
            // Plain Java objects may not carry Lume descriptors.
        } catch (IllegalAccessException | InvocationTargetException err) {
            throw new LumePanic("failed to read runtimeType: " + err.getMessage());
        }

        return LumeType.classType(
                value.getClass().getSimpleName(),
                value.getClass().getName(),
                new LumeField[] {},
                new LumeMethod[] {});
    }

    public static Object probeSuccessValue(Object value) {
        if (value instanceof Option.Some<?> some) {
            return some.value();
        }
        if (value instanceof Result.Ok<?, ?> ok) {
            return ok.value();
        }
        if (value instanceof Either.Right<?, ?> right) {
            return right.value();
        }
        return LumeUnit.INSTANCE;
    }

    public static Object extractSuccessValue(Object value) {
        if (value instanceof Option.Some<?> some) {
            return some.value();
        }
        if (value instanceof Option.None<?>) {
            throw new LumePanic("expected Option.Some");
        }
        if (value instanceof Result.Ok<?, ?> ok) {
            return ok.value();
        }
        if (value instanceof Result.Err<?, ?>) {
            throw new LumePanic("expected Result.Ok");
        }
        if (value instanceof Either.Right<?, ?> right) {
            return right.value();
        }
        if (value instanceof Either.Left<?, ?>) {
            throw new LumePanic("expected Either.Right");
        }
        throw new LumePanic("expected success value");
    }

    public static Object indexValue(Object base, Object index) {
        long offset = ((Number) index).longValue();
        Option<?> value;
        if (base instanceof LumeVector<?> list) {
            value = list.get(offset);
        } else if (base instanceof LumeArray<?> array) {
            value = array.get(offset);
        } else {
            throw new LumePanic("value is not indexable");
        }

        if (value instanceof Option.Some<?> some) {
            return some.value();
        }
        throw new LumePanic("index out of bounds: " + offset);
    }

    public static long listLen(Object value) {
        if (value instanceof LumeVector<?> list) {
            return list.size();
        }
        throw new LumePanic("expected Vector");
    }

    public static Object listGet(Object value, Object index) {
        if (value instanceof LumeVector<?> list) {
            var item = list.get(((Number) index).longValue());
            if (item instanceof Option.Some<?> some) {
                return some.value();
            }
            throw new LumePanic("vector pattern index out of bounds");
        }
        throw new LumePanic("expected Vector");
    }

    public static LumeVector<?> listSlice(Object value, Object start) {
        if (value instanceof LumeVector<?> list) {
            var javaValues = list.asJava();
            var offset = Math.max(0, ((Number) start).intValue());
            if (offset >= javaValues.size()) {
                return LumeVector.empty();
            }
            return LumeVector.from(javaValues.subList(offset, javaValues.size()));
        }
        throw new LumePanic("expected Vector");
    }

    public static <T> LumeIterator<T> iterInit(Object source) {
        return LumeIterator.from(source);
    }

    public static Boolean iterHasNext(Object iterator) {
        return ((LumeIterator<?>) iterator).hasNext();
    }

    @SuppressWarnings("unchecked")
    public static <T> T iterNext(Object iterator) {
        return ((LumeIterator<T>) iterator).next();
    }

    private static String join(Object... values) {
        return Arrays.stream(values)
                .map(String::valueOf)
                .collect(Collectors.joining(" "));
    }
}
