// Real scratch binary, TUF datastore and Sigstore verification. No test hooks.
// Run from the repository root with Node >=22.18 and frontend dependencies installed.
import assert from 'node:assert/strict';
import { execFileSync, spawnSync } from 'node:child_process';
import {
  machineMigrationCommand,
  machineCompanionCommand,
} from '../../frontend/src/schemas/machines.ts';

const updater = process.env.UPDATER_PRODUCTION_IMAGE ?? 'nyxid-machine-updater:production-test';
const testPublished = process.env.NYXID_TEST_PUBLISHED_UPDATER === '1';
const release = '0.41.0'; // Published, attested release independent of the pending patch tag.
const repository = 'ghcr.io/chronoaiproject/nyxid';
const volumePath = '/var/lib/nyxid-machine-update';
const name = `nyxid-updater-production-${process.pid}-${Date.now()}`;
const volumes = [`${name}-with-tmp`, `${name}-without-tmp`];
const docker = (...args) => execFileSync('docker', args, { encoding: 'utf8', maxBuffer: 1024 * 1024, stdio: ['ignore', 'pipe', 'pipe'] }).trim();
const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
const constraints = ['--read-only', '--cap-drop=ALL', '--security-opt=no-new-privileges', '--mount', 'type=bind,src=/var/run/docker.sock,dst=/var/run/docker.sock'];
function volume(name) { return ['--mount', `type=volume,src=${name},dst=${volumePath}`]; }
function mailbox(updateVolume, command) {
  return docker('run', '--rm', ...volume(updateVolume), 'busybox:latest', 'sh', '-c', command);
}

// Change exactly one unquoted image token and optionally forward the token by
// name. Every other byte, including all rendered flags and quoting, survives.
function productionCommand(rendered, pinned) {
  assert.match(updater, /^[a-zA-Z0-9][a-zA-Z0-9._/:@-]*$/, 'Updater must be one safe Docker image token');
  assert.ok(rendered.startsWith('docker run '));
  const originalToken = ` ${pinned} `;
  assert.equal(rendered.split(originalToken).length, 2, 'Expected exactly one rendered image token');
  const replacement = `${process.env.GITHUB_TOKEN ? ' --env GITHUB_TOKEN' : ''} ${updater} `;
  const command = rendered.replace(originalToken, replacement);
  assert.equal(command.split(replacement).length, 2, 'Expected exactly one replacement');
  assert.equal(command.replace(replacement, originalToken), rendered, 'Only image and token forwarding may change');
  return command;
}

async function renderedChecks(pinned, published) {
  const variant = published ? 'published 0.41.0 updater' : 'new production updater';
  const fixtureName = `${name}-${published ? 'published' : 'built'}`;
  const updateVolume = `${fixtureName}-nyxid-update`;
  volumes.push(updateVolume);
  // Only the fixture's health/reconnect marker is synthetic. Image provenance,
  // trust-root verification and the updater binary are real.
  docker('run', '-d', '--name', fixtureName, '--label', `dev.nyxid.machine=${fixtureName}`,
    ...volume(updateVolume), '--health-cmd', 'true', '--health-interval', '1s', '--health-timeout', '1s',
    '--entrypoint', '/bin/sh', `${repository}/nyxid-node-machine:${release}`, '-c',
    `while true; do printf '{"version":"${release}","at_ms":%s000}' "$(date +%s)" > ${volumePath}/connected.json; sleep 1; done`);
  for (const [action, render] of [['bootstrap', machineMigrationCommand], ['watch', machineCompanionCommand]]) {
    const rendered = render(fixtureName, release, pinned);
    assert.ok(rendered.includes('--tmpfs /tmp:rw,noexec,nosuid,size=16m'));
    const command = published ? rendered : productionCommand(rendered, pinned);
    const result = spawnSync('/bin/sh', ['-c', command], { encoding: 'utf8', timeout: 180_000 });
    assert.equal(result.status, 0, `Rendered ${action} command failed (${variant}): ${result.stderr}`);
    const companion = JSON.parse(docker('inspect', `${fixtureName}-updater`))[0];
    assert.equal(companion.HostConfig.ReadonlyRootfs, true);
    assert.deepEqual(companion.HostConfig.CapDrop, ['ALL']);
    assert.ok(companion.HostConfig.SecurityOpt.some((s) => s.startsWith('no-new-privileges')));
    assert.equal(companion.HostConfig.Tmpfs['/tmp'], 'rw,noexec,nosuid,size=16m');
    if (action === 'bootstrap') {
      // Bootstrap installs the official companion for the target release.
      // Replace it with the rendered watch command before issuing any request.
      assert.ok(companion.Config.Image.startsWith(`${repository}/nyxid-machine-updater@sha256:`));
      docker('rm', '-f', '-v', `${fixtureName}-updater`);
    } else {
      assert.equal(companion.Config.Image, published ? pinned : updater);
      if (!published && process.env.GITHUB_TOKEN) {
        assert.ok(companion.Config.Env.some((entry) => entry === `GITHUB_TOKEN=${process.env.GITHUB_TOKEN}`),
          'New companion must receive the token; never print its environment');
      }
    }
    checks++;
    console.log(`PASS: rendered ${action === 'bootstrap' ? 'migration' : 'companion'} command, ${variant}`);
  }
  // This private fixture volume has no earlier connected progress to mistake
  // for completion. Its newly started watch process verifies and replaces it.
  mailbox(updateVolume, `printf '${release}' > ${volumePath}/request`);
  const deadline = Date.now() + 180_000;
  let progress;
  do {
    await sleep(500);
    try { progress = JSON.parse(mailbox(updateVolume, `cat ${volumePath}/progress.json`)); } catch { /* first poll */ }
    assert.ok(!['failed', 'rolled_back'].includes(progress?.phase), `Watch failed: ${JSON.stringify(progress)}`);
  } while (progress?.phase !== 'connected' && Date.now() < deadline);
  assert.equal(progress?.phase, 'connected', `${variant} verified, replaced and reconnected`);
  checks++;
  console.log(`PASS: watch-triggered real verification and replacement, ${variant}`);
}
let checks = 0;
try {
  assert.ok(process.env.CI !== 'true' || process.env.GITHUB_TOKEN, 'CI requires GITHUB_TOKEN for authenticated attestation checks');
  if (!testPublished) console.log('SKIP (opt-in): published 0.41.0 updater uses unauthenticated GitHub API');
  if (!process.env.UPDATER_PRODUCTION_IMAGE) {
    execFileSync('docker', ['build', '--target', 'production', '-f', 'cli/Dockerfile.machine-updater', '-t', updater, '.'], { stdio: 'inherit' });
  } else {
    // Publish Images passes a pushed digest that is not local yet. Pull it
    // first: an implicit pull would print progress on the stderr that the
    // single-line failure checks below compare exactly.
    if (spawnSync('docker', ['image', 'inspect', updater], { stdio: 'ignore' }).status !== 0) {
      docker('pull', '--quiet', updater);
    }
  }
  // Run the actual entry point: one fixed stderr line, identical to progress,
  // including a connect failure and a 404 (never Docker response metadata).
  for(const [targetName, socket, reason] of [
    ['SECRET/invalid',true,'invalid_container_name'],
    [`${name}-absent`,true,'container_not_found'],
    [`${name}-absent`,false,'docker_socket_unavailable'],
  ]){
    const flags=socket?constraints:['--read-only','--cap-drop=ALL','--security-opt=no-new-privileges'];
    const result=spawnSync('docker',['run','--rm',...flags,...volume(volumes[0]),updater,'bootstrap',targetName,release],{encoding:'utf8'});
    assert.equal(result.status,1);
    assert.equal(result.stdout,'');
    assert.equal(result.stderr.trim(),`machine_update failed at bootstrap: ${reason}`);
    const progress=JSON.parse(docker('run','--rm',...volume(volumes[0]),'busybox:latest','cat',`${volumePath}/progress.json`));
    assert.equal(progress.code,`bootstrap:${reason}`);
    console.log(`PASS: production single-line failure and persisted code, ${reason}`);
    checks++;
  }
  for (const [index, tmp] of [true, false].entries()) {
    const result = docker('run', '--rm', ...constraints, ...volume(volumes[index]),
      ...(tmp ? ['--tmpfs', '/tmp:rw,noexec,nosuid,size=16m'] : []),
      ...(process.env.GITHUB_TOKEN ? ['--env', 'GITHUB_TOKEN'] : []), updater, 'verify', release);
    assert.match(result, /verified updater and machine image provenance/);
    const store = docker('run', '--rm', ...volume(volumes[index]), 'busybox:latest', 'sh', '-c',
      `test "$(stat -c %a ${volumePath}/tuf)" = 700 && test "$(stat -c %u ${volumePath}/tuf)" = 0 && find ${volumePath}/tuf -type f`);
    assert.ok(store.length > 0, 'tough persisted authenticated metadata inside the update volume');
    console.log(`PASS: production real TUF + two image attestations, ${tmp ? 'documented tmpfs' : 'no /tmp or writable root'}`);
    checks++;
  }

  // The schema requires an official pinned digest. Render with the published
  // pin, then assert the sole image-token substitution for the default checks.
  for (const image of [`${repository}/nyxid-machine-updater:${release}`, `${repository}/nyxid-node-machine:${release}`, 'busybox:latest']) {
    docker('pull', image);
  }
  const digests = JSON.parse(docker('image', 'inspect', `${repository}/nyxid-machine-updater:${release}`, '--format', '{{json .RepoDigests}}'));
  const pinned = digests.find((value) => value.startsWith(`${repository}/nyxid-machine-updater@sha256:`));
  assert.ok(pinned);
  // Assert the transformation rejects ambiguous/missing image tokens as well.
  const rendered = machineMigrationCommand(`${name}-assert`, release, pinned);
  assert.throws(() => productionCommand(`${rendered} ${pinned} `, pinned));
  assert.throws(() => productionCommand(rendered.replace(pinned, 'missing-image'), pinned));
  await renderedChecks(pinned, false);
  if (testPublished) await renderedChecks(pinned, true);
  console.log(`Production updater: ${checks} checks passed`);
} catch (error) {
  // Tests use no machine credentials; production stderr must stay fixed-code only.
  console.error(error.message);
  process.exitCode = 1;
} finally {
  for (const id of docker('ps', '-aq', '--filter', `name=${name}`).split('\n').filter(Boolean)) {
    spawnSync('docker', ['rm', '-f', '-v', id], { stdio: 'ignore' });
  }
  for (const vol of volumes) spawnSync('docker', ['volume', 'rm', vol], { stdio: 'ignore' });
}
