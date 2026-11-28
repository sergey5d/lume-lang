import { readFileSync } from 'node:fs';

export function lumeKeywords() {
  const lexer = readFileSync(new URL('../../rust/crates/lume/src/lexer.rs', import.meta.url), 'utf8');
  const words = [...lexer.matchAll(/"([a-z]+)"\s*=>\s*TokenKind::Keyword\(Keyword::\w+\)/g)].map(match => match[1]);
  if (!words.length) throw new Error('Could not read the Lume lexer keyword table.');
  return words.sort();
}
