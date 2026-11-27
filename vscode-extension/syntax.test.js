const assert = require('node:assert/strict');
const grammar = require('./syntaxes/lume.tmLanguage.json');
const configuration = require('./language-configuration.json');

function keywordScope(source) {
    for (const pattern of grammar.repository.keywords.patterns) {
        if (!pattern.name || !pattern.match) continue;
        const match = new RegExp(pattern.match).exec(source);
        if (match?.index === 0) return pattern.name;
    }
    return undefined;
}

for (const source of ['fn normalize(value Str)', 'fn +(other Vec)', 'fn [](index Int)']) {
    assert.equal(keywordScope(source), 'keyword.declaration.function.lume');
}
for (const source of ['fn(Int) Str', 'fn (Int) Str', 'fn() Int']) {
    assert.equal(keywordScope(source), 'storage.type.function.lume');
}
assert.equal(keywordScope('def normalize(value Str)'), undefined);
for (const source of ['if ready', 'then run()', 'else stop()', 'while ready', 'for item', 'do run()']) {
    assert.equal(keywordScope(source), 'keyword.control.lume');
}
for (const source of ['mix FirstParts as first', 'mixin FirstParts', 'as first']) {
    assert.equal(keywordScope(source), 'keyword.declaration.lume');
}
const builtinType = new RegExp(grammar.repository.types.patterns[0].match);
assert.ok(builtinType.test('Char'));
assert.ok(!builtinType.test('Rune'));
assert.match('fn main() {', new RegExp(configuration.folding.markers.start));
console.log('Lume syntax checks passed');
