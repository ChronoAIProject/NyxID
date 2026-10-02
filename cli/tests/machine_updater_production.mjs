// Real scratch binary, TUF datastore and Sigstore verification. No test hooks.
// Run from the repository root with Node >=22.18 and frontend dependencies installed.
import assert from 'node:assert/strict';
import { execFileSync, spawnSync } from 'node:child_process';
import {
  machineMigrationCommand,
  machineCompanionCommand,
} from '../../frontend/src/schemas/machines.ts';

const updater = process.env.UPDATER_PRODUCTION_IMAGE ?? 'nyxid-machine-updater:production-test';
const release = '0.41.0'; // Published, attested release independent of the pending patch tag.
const repository = 'ghcr.io/chronoaiproject/nyxid';
const volumePath = '/var/lib/nyxid-machine-update';
const name = `nyxid-updater-production-${process.pid}-${Date.now()}`;
const volumes = [`${name}-with-tmp`, `${name}-without-tmp`, `${name}-nyxid-update`];
const docker = (...args) => execFileSync('docker', args, { encoding: 'utf8', maxBuffer: 1024 * 1024, stdio: ['ignore', 'pipe', 'pipe'] }).trim();
const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
const constraints = ['--read-only', '--cap-drop=ALL', '--security-opt=no-new-privileges', '--mount', 'type=bind,src=/var/run/docker.sock,dst=/var/run/docker.sock'];
function volume(name) { return ['--mount', `type=volume,src=${name},dst=${volumePath}`]; }
function mailbox(command) {
  return docker('run', '--rm', ...volume(volumes[2]), 'busybox:latest', 'sh', '-c', command);
}
let checks = 0;
try {
  if (!process.env.UPDATER_PRODUCTION_IMAGE) {
    execFileSync('docker', ['build', '--target', 'production', '-f', 'cli/Dockerfile.machine-updater', '-t', updater, '.'], { stdio: 'inherit' });
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

  // Exercise the exact rendered commands with the already-published updater.
  // This catches omissions in the compatibility command even before 0.41.1 ships.
  for (const image of [`${repository}/nyxid-machine-updater:${release}`, `${repository}/nyxid-node-machine:${release}`, 'busybox:latest']) {
    docker('pull', image);
  }
  const digests = JSON.parse(docker('image', 'inspect', `${repository}/nyxid-machine-updater:${release}`, '--format', '{{json .RepoDigests}}'));
  const pinned = digests.find((value) => value.startsWith(`${repository}/nyxid-machine-updater@sha256:`));
  assert.ok(pinned);
  // A disposable official-image fixture supplies the existing machine's health
  // and reconnect marker. TUF, registry, attestation and production updater are real.
  docker('run', '-d', '--name', name, '--label', `dev.nyxid.machine=${name}`,
    ...volume(volumes[2]), '--health-cmd', 'true', '--health-interval', '1s', '--health-timeout', '1s',
    '--entrypoint', '/bin/sh', `${repository}/nyxid-node-machine:${release}`, '-c',
    `while true; do printf '{"version":"${release}","at_ms":%s000}' "$(date +%s)" > ${volumePath}/connected.json; sleep 1; done`);
  for (const command of [machineMigrationCommand(name, release, pinned), machineCompanionCommand(name, release, pinned)]) {
    assert.ok(command.includes('--tmpfs /tmp:rw,noexec,nosuid,size=16m'));
    // No rewriting/shimming arguments: run exactly what the owner copies.
    const result = spawnSync('/bin/sh', ['-c', command], { encoding: 'utf8', timeout: 180_000 });
    assert.equal(result.status, 0, `Rendered command failed: ${result.stderr}`);
    const companion = JSON.parse(docker('inspect', `${name}-updater`))[0];
    assert.equal(companion.HostConfig.ReadonlyRootfs, true);
    assert.deepEqual(companion.HostConfig.CapDrop, ['ALL']);
    assert.ok(companion.HostConfig.SecurityOpt.some((s) => s.startsWith('no-new-privileges')));
    assert.equal(companion.HostConfig.Tmpfs['/tmp'], 'rw,noexec,nosuid,size=16m');
    assert.ok(companion.Config.Image.includes('@sha256:'));
    checks++;
    if (command.includes(' bootstrap ')) {
      docker('rm', '-f', '-v', `${name}-updater`);
      console.log('PASS: rendered migration command, published 0.41.0 updater');
    }
  }
  // Trigger real verification through watch as well as bootstrap. Recreating
  // this fixture must preserve its entrypoint, health check and private volume.
  mailbox(`printf '${release}' > ${volumePath}/request`);
  const deadline = Date.now() + 180_000;
  let progress;
  do {
    await sleep(500);
    try { progress = JSON.parse(mailbox(`cat ${volumePath}/progress.json`)); } catch { /* first poll */ }
    assert.ok(!['failed', 'rolled_back'].includes(progress?.phase), `Watch failed: ${JSON.stringify(progress)}`);
  } while (progress?.phase !== 'connected' && Date.now() < deadline);
  assert.equal(progress?.phase, 'connected', 'published companion verified, replaced and reconnected');
  console.log('PASS: rendered companion command + real verification and replacement');
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
