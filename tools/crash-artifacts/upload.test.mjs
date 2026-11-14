import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import os from 'node:os';
import { spawnSync } from 'node:child_process';
import { upload } from './upload.mjs';

test('artifact upload is optional and partial configuration fails closed', () => {
  const execute = () => assert.fail('Unconfigured upload must not invoke a process');
  upload('/absent', 'web', {}, execute);
  assert.throws(() => upload('/absent', 'web', { CRASH_ARTIFACTS_SENTRY_ORG: 'example' }, execute), /Incomplete crash artifact upload configuration/);
});

test('web, iOS and Android upload commands match the pinned CLI and keep tokens out of arguments', () => {
  const directory = fs.mkdtempSync(path.join(os.tmpdir(), 'nyx-artifact-proof-'));
  const manifest = { app_version: '1.2.3', app_build: '456', artifact_id: '2e6e467d-54b7-4839-bfb1-c3e563368b37' };
  const token = 'SECRET_SENTINEL';
  const environment = { CRASH_ARTIFACTS_SENTRY_AUTH_TOKEN: token, CRASH_ARTIFACTS_SENTRY_ORG: 'example', CRASH_ARTIFACTS_SENTRY_PROJECT_WEB: 'web', CRASH_ARTIFACTS_SENTRY_PROJECT_MOBILE: 'mobile' };
  fs.writeFileSync(path.join(directory, 'manifest.json'), JSON.stringify(manifest));
  try {
    for (const platform of ['web', 'ios', 'android']) {
      const calls = [];
      upload(directory, platform, environment, (binary, args, options) => {
        calls.push(args);
        assert.equal(options.env.SENTRY_AUTH_TOKEN, token);
        assert.equal(options.env.SENTRY_PROJECT, platform === 'web' ? 'web' : 'mobile');
        assert.equal(options.stdio, 'pipe');
        assert.ok(!JSON.stringify(args).includes(token));
        // --help validates clap's production command/option parser without network I/O.
        const result = spawnSync(binary, [...args, '--help'], { encoding: 'utf8' });
        assert.equal(result.status, 0, result.stderr);
        return { status: 0 };
      });
      const release = platform === 'web' ? `nyxid-web@1.2.3+${manifest.artifact_id}` : 'nyxid-mobile@1.2.3+456';
      assert.deepEqual(calls[0], ['releases', 'new', release]);
      assert.deepEqual(calls[1], ['sourcemaps', 'upload', directory, '--release', release, '--dist', '456', '--url-prefix', 'app:///', '--validate', '--strict']);
      assert.equal(calls.length, platform === 'ios' ? 4 : 3);
      if (platform === 'ios') assert.deepEqual(calls[2], ['debug-files', 'upload', path.join(directory, 'dSYMs')]);
      assert.deepEqual(calls.at(-1), ['releases', 'finalize', release]);
    }
    assert.throws(() => upload(directory, 'web', environment, () => ({ status: 1, stderr: token })), error => error.message === 'Crash artifact upload failed' && !String(error).includes(token));
  } finally {
    fs.rmSync(directory, { recursive: true, force: true });
  }
});
