import {after, before, test} from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import {execFileSync} from 'node:child_process';
import {createRequire} from 'node:module';
import {once} from 'node:events';
import {fileURLToPath} from 'node:url';
import {startFixture} from './machine_docker_fixture.mjs';
const {WebSocket} = createRequire(new URL('../cli/tests/updater-npm/package.json', import.meta.url))('ws');
let directory, certificate, ca, key;
before(() => {
  directory = fs.mkdtempSync(path.join(os.tmpdir(), 'nyxid-local-wss-'));
  execFileSync('python3', [fileURLToPath(new URL('./machine_fixture_tls.py', import.meta.url)), directory, '127.0.0.1']);
  ca = fs.readFileSync(path.join(directory, 'ca.pem'));
  certificate = fs.readFileSync(path.join(directory, 'tls.pem'));
  key = fs.readFileSync(path.join(directory, 'tls.key'));
});
after(() => fs.rmSync(directory, {recursive: true, force: true}));
async function connect(fixture) {
  const socket = new WebSocket(`wss://127.0.0.1:${fixture.port}/api/v1/nodes/ws`, {ca});
  await once(socket, 'open');
  return socket;
}
async function send(socket, value) {
  const response = once(socket, 'message');
  socket.send(JSON.stringify(value));
  return JSON.parse((await response)[0].toString());
}
async function register(fixture) {
  const socket = await connect(fixture);
  const result = await send(socket, {type: 'register', token: fixture.token});
  socket.close();
  return result;
}

test('actual local WSS registration/authentication/status/heartbeat protocol', async t => {
  const fixture = await startFixture({certificate, key}); t.after(fixture.close);
  const identity = await register(fixture);
  assert.equal(identity.type, 'register_ok');
  const socket = await connect(fixture);
  assert.equal((await send(socket, {type: 'auth', node_id: identity.node_id, token: identity.auth_token})).type, 'auth_ok');
  socket.send(JSON.stringify({type: 'status_update', capabilities: {machine: {shell: true, files: true, browser: false,
    computer: false, separation_choice: 'shared_legacy', accidental_secret: 'must not retain'}}}));
  const heartbeat = JSON.parse((await once(socket, 'message'))[0].toString());
  assert.equal(heartbeat.type, 'heartbeat_ping');
  socket.send(JSON.stringify({type: 'heartbeat_pong'}));
  await new Promise(resolve => setTimeout(resolve, 30));
  assert.equal(fixture.state.registrations, 1); assert.equal(fixture.state.authentications, 1);
  assert.equal(fixture.state.statuses, 1); assert.equal(fixture.state.heartbeats, 1); assert.equal(fixture.state.errors, 0);
  assert.equal(fixture.state.machine.separation_choice, 'shared_legacy');
  assert(!JSON.stringify(fixture.state).includes('must not retain'));
});
test('incorrect registration token is rejected without credential delivery', async t => {
  const fixture = await startFixture({certificate, key}); t.after(fixture.close);
  const socket = await connect(fixture); const closed = once(socket, 'close');
  socket.send(JSON.stringify({type: 'register', token: 'invalid synthetic input'}));
  assert.equal((await closed)[0], 1008); assert.equal(fixture.state.registrations, 0); assert.equal(fixture.state.errors, 1);
});
test('duplicate registration is rejected', async t => {
  const fixture = await startFixture({certificate, key}); t.after(fixture.close);
  await register(fixture); const socket = await connect(fixture); const closed = once(socket, 'close');
  socket.send(JSON.stringify({type: 'register', token: fixture.token}));
  assert.equal((await closed)[0], 1008); assert.equal(fixture.state.registrations, 1); assert.equal(fixture.state.registration_attempts, 2);
});
test('authentication without registration cannot pass', async t => {
  const fixture = await startFixture({certificate, key}); t.after(fixture.close);
  const socket = await connect(fixture); const closed = once(socket, 'close');
  socket.send(JSON.stringify({type: 'auth', node_id: fixture.state.node_id, token: 'synthetic unknown'}));
  assert.equal((await closed)[0], 1008); assert.equal(fixture.state.authentications, 0);
});
test('unauthenticated capability reports cannot prove startup', async t => {
  const fixture = await startFixture({certificate, key}); t.after(fixture.close);
  const socket = await connect(fixture); const closed = once(socket, 'close');
  socket.send(JSON.stringify({type: 'status_update', capabilities: {machine: {shell: true}}}));
  assert.equal((await closed)[0], 1008); assert.equal(fixture.state.statuses, 0);
});
test('binary and malformed JSON frames are rejected', async t => {
  const fixture = await startFixture({certificate, key}); t.after(fixture.close);
  for (const value of [Buffer.from('binary frame'), '{malformed']) {
    const socket = await connect(fixture); const closed = once(socket, 'close'); socket.send(value);
    assert.equal((await closed)[0], 1008);
  }
  assert.equal(fixture.state.errors, 2);
});
test('fixture state never exposes registration/auth/signing material', async t => {
  const fixture = await startFixture({certificate, key}); t.after(fixture.close);
  const identity = await register(fixture); const state = JSON.stringify(fixture.state);
  for (const value of [fixture.token, identity.auth_token, identity.signing_secret]) assert(!state.includes(value));
});
