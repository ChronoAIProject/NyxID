// From repository root, after the shared simulator is released:
// mkdir ~/.heca/plans/pr1866-native-machines/sim.lock && \
//   /opt/homebrew/bin/node mobile/src/features/machines/desktop/qa/verifyNativeTransport.mjs
// Verifies the native transport with a synthetic source, without backend auth.
import assert from 'node:assert/strict';
import { randomUUID } from 'node:crypto';
import { createRequire } from 'node:module';
import { writeFile, unlink, rmdir } from 'node:fs/promises';
import { RED_JPEG, GREEN_PATCH, BLUE_JPEG, SESSION, vector } from '../testVectors.ts';
const require = createRequire(new URL('../../../../../package.json', import.meta.url));
const WebSocket = require('ws');
const WebSocketServer = WebSocket.Server;
const lock = '/Users/chronoai/.heca/plans/pr1866-native-machines/sim.lock';
let ownsLock = false;
let debuggerSocket;
let source;
let abort;
const timers = new Set();

function nativeTransport(token) {
  return new Promise((resolve, reject) => {
    const entry = Array.from(globalThis.__r.getModules()).find(([, value]) => value.verboseName === 'src/features/machines/desktop/viewer.ts');
    if (!entry) { reject(new Error('Production viewer module not loaded')); return; }
    const { createDesktopViewer } = globalThis.__r(entry[0]);
    const frames = [];
    let clearCount = 0;
    let idleWatching = false;
    let phase;
    let stop;
    let refreshTimer;
    let idleTimer;
    const deadline = setTimeout(() => finish(new Error('Native transport deadline')), 45000);
    const finish = error => {
      clearTimeout(deadline); clearTimeout(refreshTimer); clearTimeout(idleTimer); stop?.();
      delete globalThis.__nyxNativeTransportAbort;
      if (error) reject(error); else resolve({ frames, clearCount, idleWatching, terminalPhase: phase });
    };
    const viewer = createDesktopViewer({
      current: () => true,
      connect: () => Promise.resolve(new WebSocket('ws://127.0.0.1:4635/desktop?conversation_id=qa&context_id=qa-context&display=dev', [],
        { headers: { Authorization: 'Bearer ' + token } })),
      clear: () => clearCount++,
      frame: frame => {
        frames.push({ sequence: frame.sequence.toString(), width: frame.width, height: frame.height, base: frame.rectangle?.base.toString() });
        if (frames.length === 3) {
          viewer.refresh(); viewer.refresh();
          refreshTimer = setTimeout(() => viewer.refresh(), 1200);
          idleTimer = setTimeout(() => { idleWatching = phase === 'watching' && frames.length === 3; }, 31000);
        }
      },
      state: value => {
        phase = value.phase;
        if (phase === 'unavailable') {
          if (!idleWatching || frames.length !== 3) { finish(new Error('Viewer ended before the idle heartbeat proof')); return; }
          setTimeout(() => finish(), 1600);
        }
      },
    });
    stop = viewer.stop;
    globalThis.__nyxNativeTransportAbort = () => finish(new Error('Native transport aborted'));
    viewer.start();
  });
}

try {
  await writeFile(lock + '/owner', 'B native websocket transport proof ' + new Date().toISOString(), { flag: 'wx' }); ownsLock = true;
  const token = randomUUID();
  const received = [];
  const headers = [];
  let connections = 0;
  source = new WebSocketServer({ host: '127.0.0.1', port: 4635 });
  await new Promise((resolve, reject) => { source.once('listening', resolve); source.once('error', reject); });
  const full = vector(new Uint8Array(Buffer.from(RED_JPEG, 'base64')), 1n);
  const patch = vector(new Uint8Array(Buffer.from(GREEN_PATCH, 'base64')), 2n,
    { width: 8, height: 6, x: 2, y: 2, patchWidth: 2, patchHeight: 2, base: 1n });
  const reset = vector(new Uint8Array(Buffer.from(BLUE_JPEG, 'base64')), 3n);
  source.on('connection', (socket, request) => {
    connections++;
    headers.push({ authorizationMatches: request.headers.authorization === 'Bearer ' + token,
      origin: request.headers.origin ?? null, hasCookie: !!request.headers.cookie, path: request.url });
    socket.on('message', (bytes, binary) => {
      let message; try { message = JSON.parse(bytes.toString()); } catch { message = { malformed: true }; }
      received.push({ binary: typeof binary === 'boolean' ? binary : typeof bytes !== 'string', message, at: Date.now() });
    });
    socket.send(JSON.stringify({ type: 'connected', session_id: SESSION, controller: 'agent' }));
    for (const bytes of [full, patch, reset]) socket.send(bytes, { binary: true });
    const heartbeat = setInterval(() => { if (socket.readyState === WebSocket.OPEN) socket.send(JSON.stringify({ type: 'state', controller: 'agent' })); }, 5000);
    const close = setTimeout(() => socket.close(1008, 'Synthetic policy refusal'), 33000);
    timers.add(heartbeat); timers.add(close);
    socket.once('close', () => { clearInterval(heartbeat); clearTimeout(close); });
  });
  const pages = await (await fetch('http://localhost:8091/json/list')).json();
  const page = pages.find(value => value.description.startsWith('React Native Bridgeless'));
  assert.ok(page, 'Running native development application');
  debuggerSocket = new WebSocket(page.webSocketDebuggerUrl);
  await new Promise((resolve, reject) => { debuggerSocket.once('open', resolve); debuggerSocket.once('error', reject); });
  let serial = 0;
  const evaluate = expression => new Promise((resolve, reject) => {
    const id = ++serial;
    const message = raw => {
      const response = JSON.parse(raw); if (response.id !== id) return;
      clearTimeout(timer); debuggerSocket.off('message', message);
      if (response.result?.exceptionDetails) reject(new Error(response.result.exceptionDetails.text));
      else resolve(response.result?.result?.value);
    };
    const timer = setTimeout(() => { debuggerSocket.off('message', message); reject(new Error('Debugger request deadline')); }, 15000);
    debuggerSocket.on('message', message);
    debuggerSocket.send(JSON.stringify({ id, method: 'Runtime.evaluate', params: { expression, returnByValue: true } }));
  });
  abort = () => evaluate("if(typeof globalThis.__nyxNativeTransportAbort==='function')globalThis.__nyxNativeTransportAbort();void 0");
  const expression = require('@babel/core').transformSync(
    'globalThis.__nyxNativeTransportResult=undefined;(' + nativeTransport.toString() + ')(' + JSON.stringify(token) + ')' +
      '.then(value=>{globalThis.__nyxNativeTransportResult={ok:true,value}},error=>{globalThis.__nyxNativeTransportResult={ok:false,error:String(error)}});void 0',
    { configFile: false, babelrc: false, sourceType: 'script', presets: [[require('@react-native/babel-preset'),
      { dev: true, unstable_transformProfile: 'hermes-stable', enableBabelRuntime: false }]] },
  ).code;
  await evaluate(expression);
  let result;
  const deadline = Date.now() + 55000;
  while (Date.now() < deadline) {
    result = await evaluate('globalThis.__nyxNativeTransportResult');
    if (result) break;
    await new Promise(resolve => setTimeout(resolve, 500));
  }
  assert.ok(result?.ok, result?.error ?? 'Native transport proof deadline');
  await evaluate('delete globalThis.__nyxNativeTransportResult');
  assert.equal(connections, 1, 'Terminal policy close cannot reconnect');
  assert.equal(headers[0].authorizationMatches, true);
  assert.equal(result.value.idleWatching, true);
  assert.deepEqual(result.value.frames.map(value => [value.sequence, value.width, value.height, value.base]),
    [['1', 8, 6, undefined], ['2', 2, 2, '1'], ['3', 4, 3, undefined]]);
  assert.equal(received.length, 2, 'Immediate repeated refresh is throttled');
  assert.ok(received[1].at - received[0].at >= 1000);
  for (const value of received) { assert.equal(value.binary, false); assert.deepEqual(value.message, { type: 'refresh_frame' }); }
  console.log(JSON.stringify({ ...result.value, connections, headers, applicationMessages: received }, null, 2));
} finally {
  try {
    if (debuggerSocket?.readyState === WebSocket.OPEN) await abort?.().catch(() => {});
    debuggerSocket?.close();
    for (const timer of timers) { clearTimeout(timer); clearInterval(timer); }
    for (const socket of source?.clients ?? []) socket.terminate();
    source?.close(() => {});
  } finally {
    if (ownsLock) { await unlink(lock + '/owner'); await rmdir(lock); }
  }
}
