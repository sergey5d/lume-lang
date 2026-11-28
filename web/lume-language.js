export function lumeParser(keywords) {
  const reserved = new Set(keywords);
  function multilineString(stream, state) {
    const end = stream.string.indexOf('"""', stream.pos);
    if (end < 0) stream.skipToEnd();
    else {
      stream.pos = end + 3;
      state.multiline = false;
    }
    return 'string';
  }
  return {
    name: 'lume',
    startState: () => ({ multiline: false }),
    languageData: { commentTokens: { line: '#' } },
    token(stream, state) {
      if (state.multiline) return multilineString(stream, state);
      if (stream.eatSpace()) return null;
      if (stream.eat('#')) {
        stream.skipToEnd();
        return 'lineComment';
      }
      if (stream.match(/(?:raw)?"""/)) {
        state.multiline = true;
        return multilineString(stream, state);
      }
      if (stream.match(/(?:raw)?"/)) {
        while (!stream.eol()) {
          const ch = stream.next();
          if (ch === '\\') stream.next();
          else if (ch === '"') break;
        }
        return 'string';
      }
      if (stream.match(/[0-9][0-9_]*/)) {
        // Keep member-access dots out of numbers, just as the compiler lexer does.
        if (stream.peek() === '.' && !/^[.A-Za-z_]/.test(stream.string.slice(stream.pos + 1))) {
          stream.next();
          stream.eatWhile(/[0-9_]/);
        }
        if (stream.eat(/[eE]/)) {
          stream.eat(/[+-]/);
          stream.eatWhile(/[0-9_]/);
        }
        stream.eat('d');
        return 'number';
      }
      if (stream.match(/[A-Za-z_][A-Za-z_0-9]*/)) {
        const word = stream.current();
        if (word === 'true' || word === 'false') return 'bool';
        if (reserved.has(word)) return 'keyword';
        return /^[A-Z]/.test(word) ? 'typeName' : 'variableName';
      }
      if (stream.match(/[+\-*/%^!?<>=:&|]+/)) return 'operator';
      stream.next();
      return null;
    },
  };
}
