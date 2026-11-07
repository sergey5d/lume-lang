# String Library

Public signatures are declared in [str.lum](str.lum). For literals, escapes,
and interpolation, see [Strings](../SYNTAX.md#strings) in the syntax reference.

- `Str.size` returns the number of Unicode scalar values as `Int`; it does not
  count UTF-8 bytes or UTF-16 code units
- `Str.charAt(index)` uses a zero-based Unicode-scalar index and returns `None`
  when `index` is negative or outside the string; use `charAt(index) !` for
  assertive extraction
- `Str.slice()`, `Str.slice(start)`, and `Str.slice(start, end)` use zero-based
  Unicode-scalar indexes and return the selected half-open range; string slice
  brackets use the same operation, for example `text[:5]`, `text[1:5]`,
  `text[5:]`, and `text[:]`
- `Str.isEmpty` reports whether the string contains no characters
- `Str.nonEmpty` reports whether the string contains at least one character
- `Str.isBlank` is a getter that reports whether the string is empty or contains
  only Unicode whitespace (the Unicode `White_Space` property)
- `Str.trim()` removes leading and trailing whitespace
- `Str.trimLeft()` and `Str.trimRight()` remove whitespace from one side
- `Str.toLower()` and `Str.toUpper()` use Unicode case conversion and are not
  locale-sensitive
- `Str.contains(part)` performs literal substring membership; `contains` is the
  common membership spelling for strings and collections
- `Str.startsWith(prefix)` and `Str.endsWith(suffix)` perform literal,
  case-sensitive prefix and suffix checks
- `Str.indexOf(part)` returns the zero-based Unicode-scalar index of the first
  literal match as `Some(index)`, or `None` when no match exists
- `Str.lastIndexOf(part)` returns the index of the last literal match using the
  same optional Unicode-scalar index contract; an empty `part` matches at
  `size` (and at `0` for an empty string)
- `Str.lines()` returns a fresh, growable `Vector[Str]`, splitting on LF, CRLF,
  or CR and removing those terminators. Interior empty lines are preserved;
  a final terminator does not add an extra line, and an empty string returns
  an empty vector
- `Str.repeat(count)` and `Char.repeat(count)` return a `Str` containing the
  receiver repeated `count` times. Zero returns `""`; a negative count is a
  runtime error. Construction is subject to the runtime's string-size and
  allocation limits
- `Str.split(separator)` treats `separator` as literal text and returns a
  growable `Vector[Str]`
- `Str.splitRegex(pattern)` explicitly interprets `pattern` as a regular
  expression and returns a growable `Vector[Str]`
- `Str.replaceFirst(target, replacement)` and
  `Str.replaceAll(target, replacement)` replace literal text
- `Str.replaceFirstRegex(pattern, replacement)` and
  `Str.replaceAllRegex(pattern, replacement)` make regular-expression
  replacement explicit in the method name
