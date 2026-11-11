// One explicit browser choice for native acceptance, prereqs and real setup.
import fs from 'node:fs';
import path from 'node:path';
import {pathToFileURL} from 'node:url';

export const nativeBrowserDefault = '/usr/bin/google-chrome';
const candidates = ['/usr/bin/chromium', '/usr/bin/chromium-browser', nativeBrowserDefault];

export function browserLayout(resolved) {
  if (/^\/snap\/chromium\/\d+\/usr\/lib\/chromium-browser\/chrome$/.test(resolved)) {
    return {policyDirectory: '/etc/chromium-browser/policies/managed', browserExecutable: resolved};
  }
  if (['/usr/bin/chromium', '/usr/lib/chromium/chromium'].includes(resolved)) {
    return {policyDirectory: '/etc/chromium/policies/managed', browserExecutable: '/usr/lib/chromium/chromium'};
  }
  if (['/opt/google/chrome/google-chrome', '/opt/google/chrome/chrome'].includes(resolved)) {
    return {policyDirectory: '/etc/opt/chrome/policies/managed', browserExecutable: '/opt/google/chrome/chrome'};
  }
  throw new Error(`native acceptance requires a recognized resolved browser package layout: ${resolved}`);
}

function inspectBrowser(candidate) {
  try {
    const resolved = fs.realpathSync(candidate);
    if (!fs.statSync(candidate).isFile()) return {resolved, error: 'not a regular file'};
    fs.accessSync(candidate, fs.constants.X_OK);
    return {resolved};
  } catch (error) {
    return {error: error.code ?? error.message};
  }
}

export function selectBrowser({
  browser = process.env.NYXID_MACHINE_BROWSER ?? nativeBrowserDefault,
  inspect = inspectBrowser,
  log = console.error,
  requireGoogleChrome = false,
} = {}) {
  const observations = new Map();
  for (const candidate of new Set([...candidates, browser])) {
    const observation = inspect(candidate);
    observations.set(candidate, observation);
    log(`Native browser candidate=${JSON.stringify(candidate)} resolved=${JSON.stringify(observation.resolved ?? null)} error=${JSON.stringify(observation.error ?? null)} selected=${candidate === browser}`);
  }
  if (!path.isAbsolute(browser)) throw new Error('NYXID_MACHINE_BROWSER must be an absolute executable path');
  const {resolved, error} = observations.get(browser);
  if (error || !resolved) throw new Error(`selected native browser unavailable: ${browser}: ${error ?? 'unresolved'}`);
  const layout = browserLayout(resolved);
  if (requireGoogleChrome && layout.policyDirectory !== '/etc/opt/chrome/policies/managed') {
    throw new Error('hosted native job requires the packaged Google Chrome policy layout');
  }
  log(`Native browser selected=${JSON.stringify(browser)} resolved=${JSON.stringify(resolved)} policy_directory=${JSON.stringify(layout.policyDirectory)}`);
  return {browserBinary: browser, resolvedBrowser: resolved, ...layout};
}

if (process.argv[1] && import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href) {
  try {
    console.log(JSON.stringify(selectBrowser({requireGoogleChrome: process.argv.includes('--require-google-chrome')})));
  } catch (error) {
    console.error(error.message);
    process.exitCode = 1;
  }
}
