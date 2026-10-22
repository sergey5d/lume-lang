package lume.core;

import java.util.Map;
import java.util.regex.Pattern;
import java.util.regex.PatternSyntaxException;

public final class LumeRegex {
    private final Pattern pattern;
    private final Map<String, Integer> names;

    private LumeRegex(Pattern pattern) {
        this.pattern = pattern;
        names = pattern.namedGroups();
    }

    public static Result<LumeRegex, RegexError> compile(String source) {
        try {
            return new Result.Ok<>(new LumeRegex(Pattern.compile(
                    source, Pattern.UNICODE_CHARACTER_CLASS | Pattern.UNIX_LINES)));
        } catch (PatternSyntaxException error) {
            var offset = error.getIndex() < 0 ? LumeRuntime.<Long>optionNone()
                    : LumeRuntime.optionSome((long) source.codePointCount(
                            0, Math.min(error.getIndex(), source.length())));
            return new Result.Err<>(new RegexError(error.getDescription(), offset));
        }
    }

    public static String escape(String text) {
        var result = new StringBuilder();
        for (int index = 0; index < text.length(); index++) {
            var value = text.charAt(index);
            if ("\\.+*?()|[]{}^$#&-~".indexOf(value) >= 0) {
                result.append('\\');
            }
            result.append(value);
        }
        return result.toString();
    }

    @Override
    public String toString() { return "Regex(" + pattern.pattern() + ")"; }

    public boolean contains(String text) { return pattern.matcher(text).find(); }
    public boolean matches(String text) { return pattern.matcher(text).matches(); }

    public Option<LumeRegexMatch> find(String text) {
        var matcher = pattern.matcher(text);
        return matcher.find() ? LumeRuntime.optionSome(new LumeRegexMatch(text, matcher, names))
                : LumeRuntime.optionNone();
    }

    public Option<LumeRegexMatch> fullMatch(String text) {
        var matcher = pattern.matcher(text);
        return matcher.matches() ? LumeRuntime.optionSome(new LumeRegexMatch(text, matcher, names))
                : LumeRuntime.optionNone();
    }

    public LumeVector<LumeRegexMatch> findAll(String text) {
        var result = LumeVector.<LumeRegexMatch>empty();
        var matcher = pattern.matcher(text);
        int cursor = 0;
        int previousEnd = -1;
        while (cursor <= text.length() && matcher.find(cursor)) {
            int start = matcher.start();
            int end = matcher.end();
            // Do not split a supplementary rune or repeat an empty match at the last match's end.
            if (start != end || start != previousEnd) {
                result.add(new LumeRegexMatch(text, matcher, names));
            }
            previousEnd = end;
            if (start == end) {
                if (end == text.length()) {
                    break;
                }
                cursor = end + Character.charCount(text.codePointAt(end));
            } else {
                cursor = end;
            }
        }
        return result;
    }
}
