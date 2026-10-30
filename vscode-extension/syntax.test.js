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
assert.match('fn main() {', new RegExp(configuration.folding.markers.start));
console.log('Lume syntax checks passed');
