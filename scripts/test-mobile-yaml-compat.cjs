// Resolve the real React Native/Istanbul consumer in a clean frozen pnpm install.
const assert = require('node:assert/strict');
const { createRequire } = require('node:module');
const { spawnSync } = require('node:child_process');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const { test } = require('node:test');
const root = fs.realpathSync(process.argv[2] || path.resolve(__dirname, '../mobile'));
let consumer = createRequire(path.join(root, 'package.json'));
for (const name of ['react-native', 'babel-jest', 'babel-plugin-istanbul', '@istanbuljs/load-nyc-config']) {
  consumer = createRequire(consumer.resolve(name + '/package.json'));
}
const nyc = consumer('@istanbuljs/load-nyc-config');
const yamlManifest = consumer.resolve('js-yaml/package.json');
const yamlRequire = createRequire(yamlManifest);
const yaml = JSON.parse(fs.readFileSync(yamlManifest));
const parserPath = yamlRequire.resolve('argparse/package.json');
const cli = path.join(path.dirname(yamlManifest), 'bin/js-yaml.js');
function run(args, input) {
  return spawnSync(process.execPath, [cli, ...args], { encoding: 'utf8', input, timeout: 5000 });
}
test('actual Istanbul YAML 3 consumer uses argparse 2 without sprintf-js', () => {
  assert.ok(yaml.version.startsWith('3.'));
  assert.equal(JSON.parse(fs.readFileSync(parserPath)).version, '2.0.1');
  assert.ok(fs.realpathSync(parserPath).startsWith(root + path.sep));
  assert.throws(() => createRequire(parserPath).resolve('sprintf-js'), { code: 'MODULE_NOT_FOUND' });
  assert.doesNotMatch(fs.readFileSync(path.join(root, 'pnpm-lock.yaml'), 'utf8'), /sprintf-js@|argparse@1\./);
});
test('legacy js-yaml CLI help retains its parser options', () => {
  const result = run(['--help']);
  assert.ifError(result.error); assert.equal(result.status, 0, result.stderr);
  for (const option of ['--help', '--version', '--compact', '--trace']) assert.ok(result.stdout.includes(option));
});
test('legacy js-yaml CLI converts valid YAML to JSON', () => {
  const result = run(['--to-json'], 'answer: 42\n');
  assert.ifError(result.error); assert.equal(result.status, 0, result.stderr);
  assert.deepEqual(JSON.parse(result.stdout), { answer: 42 });
});
test('legacy js-yaml CLI rejects malformed YAML with the expected status', () => {
  const result = run(['--compact'], 'broken: [\n');
  assert.ifError(result.error); assert.equal(result.status, 1);
  assert.match(result.stderr, /YAMLException/);
});
test('actual NYC loader reads YAML configuration and preserves options', async () => {
  const directory = fs.mkdtempSync(path.join(os.tmpdir(), 'nyxid-yaml-compat-'));
  try {
    fs.writeFileSync(path.join(directory, 'package.json'), '{"private":true}');
    fs.writeFileSync(path.join(directory, '.nycrc.yaml'), 'all: true\ninclude:\n  - src/**/*.js\nreporter:\n  - text\n');
    const config = await nyc.loadNycConfig({ cwd: directory });
    assert.equal(config.all, true); assert.deepEqual(config.include, ['src/**/*.js']);
    assert.deepEqual(config.reporter, ['text']);
  } finally { fs.rmSync(directory, { recursive: true }); }
});
