package lume.runtime;

import java.util.Arrays;
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

    private static String join(Object... values) {
        return Arrays.stream(values)
                .map(String::valueOf)
                .collect(Collectors.joining(" "));
    }
}
