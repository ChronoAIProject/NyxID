import assert from 'node:assert/strict';
import {test} from 'node:test';
import {browserLayout, nativeBrowserDefault, selectBrowser} from './machine_native_browser.mjs';

const chrome = '/opt/google/chrome/google-chrome';
const snap = '/snap/chromium/123/usr/lib/chromium-browser/chrome';
function select(files, options = {}) {
  const messages = [];
  const result = selectBrowser({
    browser: nativeBrowserDefault,
    inspect: candidate => files[candidate] ? {resolved: files[candidate]} : {error: 'ENOENT'},
    log: message => messages.push(message),
    ...options,
  });
  return {result, messages};
}

test('hosted default selects Google Chrome despite installed Chromium shims', () => {
  const {result, messages} = select({
    '/usr/bin/chromium': '/usr/bin/chromium',
    '/usr/bin/chromium-browser': '/usr/bin/chromium-browser',
    '/usr/bin/google-chrome': chrome,
  }, {requireGoogleChrome: true});
  assert.equal(result.browserBinary, '/usr/bin/google-chrome');
  assert.equal(result.browserExecutable, '/opt/google/chrome/chrome');
  assert.equal(result.policyDirectory, '/etc/opt/chrome/policies/managed');
  for (const candidate of ['/usr/bin/chromium', '/usr/bin/chromium-browser', '/usr/bin/google-chrome']) {
    assert(messages.some(message => message.includes(`candidate=${JSON.stringify(candidate)}`)));
  }
  assert(messages.some(message => message.includes(`resolved=${JSON.stringify(chrome)}`) && message.includes('selected=true')));
});

test('missing default fails without selecting an installed shim', () => {
  assert.throws(() => select({'/usr/bin/chromium-browser': '/usr/bin/chromium-browser'}), /selected native browser unavailable/);
});

test('explicit selection is used with no fallback', () => {
  const files = {'/usr/bin/google-chrome': chrome, '/usr/bin/chromium': '/usr/lib/chromium/chromium'};
  assert.equal(select(files, {browser: '/usr/bin/chromium'}).result.policyDirectory, '/etc/chromium/policies/managed');
  assert.throws(() => select(files, {browser: '/missing'}), /selected native browser unavailable/);
  for (const browser of ['', 'google-chrome']) {
    assert.throws(() => select(files, {browser}), /absolute executable path/);
  }
});

test('Snap selection retains its package-correct policy directory', () => {
  const {result} = select({'/usr/bin/chromium': snap}, {browser: '/usr/bin/chromium'});
  assert.equal(result.policyDirectory, '/etc/chromium-browser/policies/managed');
  assert.equal(result.browserExecutable, snap);
});

test('hosted prerequisite rejects explicit Snap and Debian selections', () => {
  for (const resolved of [snap, '/usr/bin/chromium', '/usr/lib/chromium/chromium']) {
    assert.throws(() => select({'/usr/bin/chromium': resolved}, {
      browser: '/usr/bin/chromium', requireGoogleChrome: true,
    }), /hosted native job requires the packaged Google Chrome policy layout/);
  }
});

test('unknown layouts are refused even with a familiar executable name', () => {
  assert.throws(() => select({'/usr/bin/google-chrome': '/opt/custom/google-chrome'}), /recognized resolved browser package layout/);
  assert.throws(() => browserLayout('/snap/chromium/current/usr/lib/chromium-browser/chrome'), /recognized/);
});

test('non-executable selection fails and its diagnostic is printed', () => {
  const messages = [];
  assert.throws(() => selectBrowser({
    browser: nativeBrowserDefault,
    inspect: () => ({resolved: chrome, error: 'EACCES'}), log: message => messages.push(message),
  }), /selected native browser unavailable/);
  assert(messages.some(message => message.includes('EACCES')));
});

test('Chrome binary and launcher resolutions share one policy layout', () => {
  assert.deepEqual(browserLayout(chrome), browserLayout('/opt/google/chrome/chrome'));
});
