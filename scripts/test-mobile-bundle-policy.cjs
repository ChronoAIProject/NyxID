const assert = require('node:assert/strict');
const { test } = require('node:test');
const { sources, inspect } = require('./check-mobile-bundle.cjs');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
function map(names) { return { version: 3, sources: names, sourcesContent: names.map(() => 'source') }; }
test('direct, pnpm, relocated and Windows glob-module paths fail', () => {
  for (const source of ['../node_modules/braces/index.js', '../../node_modules/.pnpm/braces@3.0.3/node_modules/braces/lib/parse.js',
    'app/node_modules/micromatch/index.js', '..\\node_modules\\micromatch\\index.js']) {
    assert.throws(() => sources(map([source])), /glob module leaked/);
  }
});
test('nested indexed source maps cannot hide forbidden modules', () => {
  assert.throws(() => sources({ version: 3, sections: [{ offset: { line: 0, column: 0 },
    map: map(['node_modules/braces/index.js']) }] }), /glob module leaked/);
});
test('missing module inventory or source contents fails', () => {
  for (const data of [{}, { version: 3, sources: [] }, { version: 3, sources: ['src/index.ts'] }])
    assert.throws(() => sources(data));
});
test('unrelated path names remain valid', () => {
  assert.deepEqual(sources(map(['src/braces-help.tsx', 'src/micromatch-example.ts'])), ['src/braces-help.tsx', 'src/micromatch-example.ts']);
});
test('missing maps, bundles or actual app/native sources fail', () => {
  const directory = fs.mkdtempSync(path.join(os.tmpdir(), 'nyxid-bundle-policy-'));
  try {
    assert.throws(() => inspect(directory), /did not emit/);
    fs.writeFileSync(path.join(directory, 'index.js.map'), JSON.stringify(map(['unrelated.js'])));
    assert.throws(() => inspect(directory));
    fs.writeFileSync(path.join(directory, 'index.js'), 'actual bundle');
    assert.throws(() => inspect(directory), /application modules/);
    fs.writeFileSync(path.join(directory, 'index.js.map'), JSON.stringify(map(['src/index.tsx', 'node_modules/react-native/index.js'])));
    assert.deepEqual(inspect(directory), { maps: 1, modules: 2, forbiddenModules: 0 });
  } finally { fs.rmSync(directory, { recursive: true }); }
});
