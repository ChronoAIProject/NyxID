#!/usr/bin/env node
// Export the actual production application, then inspect every emitted source map.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const { spawnSync } = require('node:child_process');
const { createRequire } = require('node:module');
const forbidden = /(?:^|\/)(?:braces|micromatch)(?:@[^/]+)?(?:\/|$)/;
function sources(map) {
  assert.equal(map.version, 3, 'production source map must use version 3');
  if (map.sections) {
    assert.ok(Array.isArray(map.sections) && map.sections.length);
    return map.sections.flatMap(section => sources(section.map));
  }
  assert.ok(Array.isArray(map.sources) && map.sources.length, 'source map must enumerate bundled modules');
  assert.ok(Array.isArray(map.sourcesContent) && map.sourcesContent.length === map.sources.length,
    'source map must include source content for every module');
  return map.sources.map((source, index) => {
    assert.equal(typeof source, 'string');
    assert.equal(typeof map.sourcesContent[index], 'string');
    const normalized = source.replaceAll('\\', '/');
    assert.ok(!forbidden.test(normalized), 'build-only glob module leaked into the app: ' + normalized);
    return normalized;
  });
}
function inspect(directory) {
  const files = [];
  function walk(base) {
    for (const name of fs.readdirSync(base)) {
      const file = path.join(base, name), info = fs.lstatSync(file);
      assert.ok(!info.isSymbolicLink(), 'export must contain actual files');
      if (info.isDirectory()) walk(file); else if (file.endsWith('.map')) files.push(file);
    }
  }
  walk(directory);
  assert.ok(files.length, 'production export did not emit source maps');
  const modules = [];
  for (const file of files) {
    const bundle = file.slice(0, -4);
    assert.ok(fs.statSync(bundle).size > 0, 'each map must name an emitted bundle');
    modules.push(...sources(JSON.parse(fs.readFileSync(file))));
  }
  assert.ok(modules.some(source => /(?:^|\/)src\/.*\.[jt]sx?$/.test(source)), 'actual application modules must be present');
  assert.ok(modules.some(source => /(?:^|\/)react-native\//.test(source)), 'actual native runtime modules must be present');
  return { maps: files.length, modules: modules.length, forbiddenModules: 0 };
}
function exportAndInspect(root) {
  const output = fs.mkdtempSync(path.join(os.tmpdir(), 'nyxid-mobile-production-bundle-'));
  try {
    const resolver = createRequire(path.join(root, 'package.json'));
    const manifest = resolver.resolve('expo/package.json');
    const cli = path.join(path.dirname(manifest), 'bin/cli');
    const environment = { ...process.env, APP_ENV: 'prod', CI: 'true', EXPO_NO_TELEMETRY: '1',
      EXPO_OFFLINE: '1', NODE_ENV: 'production',
      PROD_API_BASE_URL: 'https://bundle-fixture.invalid', PROD_FRONTEND_URL: 'https://bundle-fixture.invalid',
      PROD_IOS_BUNDLE_ID: 'dev.nyxid.securitybundle', PROD_ANDROID_PACKAGE: 'dev.nyxid.securitybundle',
      PROD_IOS_BUILD_NUMBER: '1', PROD_ANDROID_VERSION_CODE: '1' };
    delete environment.NODE_OPTIONS; delete environment.NODE_PATH;
    const exported = spawnSync(process.execPath, [cli, 'export', '--platform', 'ios', '--source-maps',
      '--no-bytecode', '--max-workers', '2', '--output-dir', output],
      { cwd: root, env: environment, encoding: 'utf8', timeout: 600000 });
    assert.ifError(exported.error);
    assert.equal(exported.status, 0, exported.stdout + exported.stderr);
    const result = inspect(output);
    console.log('Actual iOS production bundle: ' + JSON.stringify(result));
    return result;
  } finally { fs.rmSync(output, { recursive: true }); }
}
module.exports = { sources, inspect, exportAndInspect };
if (require.main === module) exportAndInspect(path.resolve(process.argv[2] || path.join(__dirname, '../mobile')));
