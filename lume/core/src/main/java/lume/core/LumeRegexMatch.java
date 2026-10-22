package lume.core;

import java.util.Map;
import java.util.regex.Matcher;

public final class LumeRegexMatch {
    private final String text;
    private final long start;
    private final long end;
    private final String[] captures;
    private final Map<String, Integer> names;

    LumeRegexMatch(String input, Matcher matcher, Map<String, Integer> names) {
        text = matcher.group();
        start = input.codePointCount(0, matcher.start());
        end = input.codePointCount(0, matcher.end());
        captures = new String[matcher.groupCount() + 1];
        for (int index = 0; index < captures.length; index++) {
            captures[index] = matcher.group(index);
        }
        this.names = names;
    }

    public String text() { return text; }
    public long start() { return start; }
    public long end() { return end; }

    @Override
    public String toString() { return "RegexMatch(" + text + ")"; }

    public Option<String> group(long index) {
        if (index < 0 || index >= captures.length || captures[(int) index] == null) {
            return LumeRuntime.optionNone();
        }
        return LumeRuntime.optionSome(captures[(int) index]);
    }

    public Option<String> group(String name) {
        var index = names.get(name);
        return index == null ? LumeRuntime.optionNone() : group(index.longValue());
    }

    public LumeVector<Option<String>> groups() {
        var result = LumeVector.<Option<String>>empty();
        for (int index = 0; index < captures.length; index++) {
            result.add(group(index));
        }
        return result;
    }

    public LumeMap<String, String> groupMap() {
        var result = LumeMap.<String, String>empty();
        names.entrySet().stream().sorted(Map.Entry.comparingByValue()).forEach(entry -> {
            var value = captures[entry.getValue()];
            if (value != null) {
                result.set(entry.getKey(), value);
            }
        });
        return result;
    }
}
