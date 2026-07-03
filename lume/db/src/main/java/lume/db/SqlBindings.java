package lume.db;

import java.util.ArrayList;
import java.util.Arrays;
import java.util.Collections;
import java.util.List;
import java.util.Map;

import lume.core.LumeList;
import lume.core.LumeMap;
import lume.core.Tuple2;
import lume.core.Tuple3;
import lume.core.Tuple4;
import lume.core.Tuple5;
import lume.core.Tuple6;
import lume.core.Tuple7;
import lume.core.Tuple8;

final class SqlBindings {
    private final List<Object> positional;
    private final Map<String, ?> named;

    private SqlBindings(List<Object> positional, Map<String, ?> named) {
        this.positional = Collections.unmodifiableList(new ArrayList<>(positional));
        this.named = named == null ? null : Collections.unmodifiableMap(named);
    }

    static SqlBindings empty() {
        return new SqlBindings(List.of(), null);
    }

    static SqlBindings from(Object... values) {
        if (values.length == 1) {
            return fromOne(values[0]);
        }
        return positional(Arrays.asList(values));
    }

    static SqlBindings from(Object first, Object... rest) {
        var values = new ArrayList<Object>(rest.length + 1);
        values.add(first);
        values.addAll(Arrays.asList(rest));
        return from(values.toArray());
    }

    List<Object> positional() {
        return positional;
    }

    Map<String, ?> named() {
        return named;
    }

    private static SqlBindings fromOne(Object value) {
        if (value instanceof LumeMap<?, ?> map) {
            return namedMap(map);
        }
        if (value instanceof LumeList<?> list) {
            return positional(new ArrayList<>(list.asJava()));
        }
        if (value instanceof Tuple2<?, ?> tuple) {
            return positional(nullableList(tuple.first(), tuple.second()));
        }
        if (value instanceof Tuple3<?, ?, ?> tuple) {
            return positional(nullableList(tuple.first(), tuple.second(), tuple.third()));
        }
        if (value instanceof Tuple4<?, ?, ?, ?> tuple) {
            return positional(nullableList(tuple.first(), tuple.second(), tuple.third(), tuple.fourth()));
        }
        if (value instanceof Tuple5<?, ?, ?, ?, ?> tuple) {
            return positional(nullableList(tuple.first(), tuple.second(), tuple.third(), tuple.fourth(), tuple.fifth()));
        }
        if (value instanceof Tuple6<?, ?, ?, ?, ?, ?> tuple) {
            return positional(nullableList(
                tuple.first(),
                tuple.second(),
                tuple.third(),
                tuple.fourth(),
                tuple.fifth(),
                tuple.sixth()
            ));
        }
        if (value instanceof Tuple7<?, ?, ?, ?, ?, ?, ?> tuple) {
            return positional(nullableList(
                tuple.first(),
                tuple.second(),
                tuple.third(),
                tuple.fourth(),
                tuple.fifth(),
                tuple.sixth(),
                tuple.seventh()
            ));
        }
        if (value instanceof Tuple8<?, ?, ?, ?, ?, ?, ?, ?> tuple) {
            return positional(nullableList(
                tuple.first(),
                tuple.second(),
                tuple.third(),
                tuple.fourth(),
                tuple.fifth(),
                tuple.sixth(),
                tuple.seventh(),
                tuple.eighth()
            ));
        }
        return positional(nullableList(value));
    }

    private static SqlBindings positional(List<Object> values) {
        return new SqlBindings(values, null);
    }

    private static SqlBindings namedMap(LumeMap<?, ?> map) {
        var out = new java.util.LinkedHashMap<String, Object>();
        for (var entry : map.asJava().entrySet()) {
            var key = entry.getKey();
            if (!(key instanceof String name)) {
                throw new IllegalArgumentException("named SQL bind keys must be Str");
            }
            out.put(name, entry.getValue());
        }
        return new SqlBindings(List.of(), out);
    }

    private static List<Object> nullableList(Object... values) {
        return new ArrayList<>(Arrays.asList(values));
    }
}
