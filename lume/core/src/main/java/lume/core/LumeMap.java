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

    @SuppressWarnings("unchecked")
    public static <K, V> LumeMap<K, V> fromParts(Object... parts) {
        var map = new LinkedHashMap<K, V>();
        for (var part : parts) {
            if (part instanceof Tuple2<?, ?> entry) {
                map.put((K) entry.first(), (V) entry.second());
            } else if (part instanceof LumeMap<?, ?> spread) {
                for (var entry : spread.values.entrySet()) {
                    map.put((K) entry.getKey(), (V) entry.getValue());
                }
            } else {
                throw new IllegalArgumentException("map construction expects entries or map spreads");
            }
        }
        return new LumeMap<>(map);
    }

    public LumeMap<K, V> put(K key, V value) {
        var copy = new LinkedHashMap<>(values);
        copy.put(key, value);
        return new LumeMap<>(copy);
    }

    public void set(K key, V value) {
        values.put(key, value);
    }

    public Option<V> get(K key) {
        if (!values.containsKey(key)) {
            return LumeRuntime.optionNone();
        }
        return LumeRuntime.optionSome(values.get(key));
    }

    public long size() {
        return values.size();
    }

    public void clear() {
        values.clear();
    }

    public LumeVector<Tuple2<K, V>> entries() {
        var entries = LumeVector.<Tuple2<K, V>>empty();
        for (var entry : values.entrySet()) {
            entries.add(new Tuple2<>(entry.getKey(), entry.getValue()));
        }
        return entries;
    }

    public Map<K, V> asJava() {
        return Map.copyOf(values);
    }
}
