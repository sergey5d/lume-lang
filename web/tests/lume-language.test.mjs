import { test } from 'node:test';
import assert from 'node:assert/strict';
import { StreamLanguage } from '@codemirror/language';
import { classHighlighter, highlightTree } from '@lezer/highlight';
import { lumeParser } from '../lume-language.js';
import { lumeKeywords } from '../scripts/lume-keywords.mjs';

const language = StreamLanguage.define(lumeParser(lumeKeywords()));
function tokens(source) {
  const result = [];
  highlightTree(language.parser.parse(source), classHighlighter, (from, to, style) => {
    result.push([source.slice(from, to), style]);
  });
  return result;
}

test('every compiler keyword is highlighted, including public, get, mix, then, and do', () => {
  const words = lumeKeywords();
  assert(words.includes('public') && words.includes('get') && words.includes('mix') && words.includes('then') && words.includes('do'));
  assert.deepEqual(tokens(words.join(' ')), words.map(word => [word, word === 'true' || word === 'false' ? 'tok-bool' : 'tok-keyword']));
});

test('ordinary and removed keyword names stay identifiers', () => {
  const words = ['def', 'default', 'override', 'hidden', 'new', 'getter', 'publicValue'];
  assert.deepEqual(tokens(words.join(' ')), words.map(word => [word, 'tok-variableName']));
});

test('comments and escaped strings never highlight their contents as keywords', () => {
  assert.deepEqual(tokens('# public fn 10\n"public \\"fn\\" # 20"\npublic'), [
    ['# public fn 10', 'tok-comment'],
    ['"public \\"fn\\" # 20"', 'tok-string'],
    ['public', 'tok-keyword'],
  ]);
});

test('multiline and raw strings retain their state across lines and blank lines', () => {
  const source = 'raw"""fn\n\n# public 10\n"""\nget';
  const highlighted = tokens(source);
  assert.equal(highlighted.at(-1)[0], 'get');
  assert.equal(highlighted.at(-1)[1], 'tok-keyword');
  assert(highlighted.slice(0, -1).every(([, style]) => style === 'tok-string'));
  assert.deepEqual(tokens('raw"if \\"then\\"" else'), [
    ['raw"if \\"then\\""', 'tok-string'],
    ['else', 'tok-keyword'],
  ]);
});

test('an unterminated single-line string does not hide keywords on the next line', () => {
  assert.deepEqual(tokens('"unfinished\npublic'), [
    ['"unfinished', 'tok-string'],
    ['public', 'tok-keyword'],
  ]);
});

test('numbers include separators, exponents, and decimals but not member access', () => {
  const numbers = tokens('1_000 2.5e-6 19.99d 3d.toStr() 42.toStr()');
  assert.deepEqual(numbers.filter(([, style]) => style === 'tok-number').map(([text]) => text), ['1_000', '2.5e-6', '19.99d', '3d', '42']);
});

test('types are distinct from identifier prefixes', () => {
  assert.deepEqual(tokens('Int User publicValue mixinParts'), [
    ['Int', 'tok-typeName'],
    ['User', 'tok-typeName'],
    ['publicValue', 'tok-variableName'],
    ['mixinParts', 'tok-variableName'],
  ]);
});
