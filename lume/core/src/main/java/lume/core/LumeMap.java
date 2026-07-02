package lume.core;

import java.util.LinkedHashMap;
import java.util.Map;

public final class LumeMap<K, V> {
    private final LinkedHashMap<K, V> values;

    private LumeMap(LinkedHashMap<K, V> values) {
        this.values = values;
    }

    public static <K, V> LumeMap<K, V> empty() {
        return new LumeMap<>(new LinkedHashMap<>());
    }

    public static <K, V> LumeMap<K, V> fromEntries(Iterable<Tuple2<K, V>> entries) {
        var map = new LinkedHashMap<K, V>();
        for (var entry : entries) {
            map.put(entry.first(), entry.second());
        }
        return new LumeMap<>(map);
    }

    public LumeMap<K, V> put(K key, V value) {
        var copy = new LinkedHashMap<>(values);
        copy.put(key, value);
        return new LumeMap<>(copy);
    }

    public Option<V> get(K key) {
        if (!values.containsKey(key)) {
            return Option.none();
        }
        return Option.some(values.get(key));
    }

    public long size() {
        return values.size();
    }

    public Map<K, V> asJava() {
        return Map.copyOf(values);
    }
}
