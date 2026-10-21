// Local WSS transport only: no account APIs, credential material logs or production calls.
import https from 'node:https';
import fs from 'node:fs';
import {randomBytes, randomUUID} from 'node:crypto';
import {createRequire} from 'node:module';
import {pathToFileURL} from 'node:url';
const {WebSocketServer} = createRequire(new URL('../cli/tests/updater-npm/package.json', import.meta.url))('ws');

export async function startFixture({certificate, key, host = '127.0.0.1'}) {
  const token = `nyx_nreg_${randomBytes(32).toString('hex')}`;
  const auth = `nyx_nauth_${randomBytes(32).toString('hex')}`;
  const signing = randomBytes(32).toString('hex');
  const nodeId = randomUUID();
  const state = {connections: 0, registration_attempts: 0, registrations: 0, authentications: 0,
    statuses: 0, heartbeats: 0, requests: 0, errors: 0, node_id: nodeId, machine: null};
  const server = https.createServer({cert: certificate, key}, (request, response) => {
    if (request.url !== '/__fixture/state') state.requests++;
    response.writeHead(200, {'content-type': 'application/json', 'cache-control': 'no-store'});
    response.end(JSON.stringify(request.url === '/__fixture/state' ? state : {pending_credentials: []}));
  });
  const wss = new WebSocketServer({server, path: '/api/v1/nodes/ws', maxPayload: 64 * 1024});
  wss.on('connection', socket => {
    state.connections++;
    let authenticated = false;
    let heartbeat;
    const refuse = () => { state.errors++; socket.close(1008, 'fixture rejected'); };
    socket.on('error', () => { state.errors++; });
    socket.on('close', () => clearInterval(heartbeat));
    socket.on('message', (bytes, binary) => {
      if (binary) return refuse();
      let message;
      try { message = JSON.parse(bytes.toString()); } catch { return refuse(); }
      if (message.type === 'register') {
        state.registration_attempts++;
        if (message.token !== token || state.registrations !== 0) return refuse();
        state.registrations++;
        socket.send(JSON.stringify({type: 'register_ok', node_id: nodeId, auth_token: auth, signing_secret: signing}));
      } else if (message.type === 'auth') {
        if (message.node_id !== nodeId || message.token !== auth || state.registrations !== 1 || authenticated) return refuse();
        authenticated = true;
        state.authentications++;
        socket.send(JSON.stringify({type: 'auth_ok', heartbeat_interval_secs: 2, capabilities: {proxy_binary_chunks: true}}));
        heartbeat = setInterval(() => {
          if (socket.readyState === 1) socket.send(JSON.stringify({type: 'heartbeat_ping'}));
        }, 1000);
      } else if (message.type === 'status_update') {
        if (!authenticated || !message.capabilities?.machine) return refuse();
        state.statuses++;
        const profile = message.capabilities.machine;
        // Project only capability metadata; never retain arbitrary messages.
        state.machine = Object.fromEntries(['shell', 'files', 'computer', 'browser', 'separated', 'separation_evidence', 'separation_choice']
          .filter(name => Object.hasOwn(profile, name)).map(name => [name, profile[name]]));
      } else if (message.type === 'heartbeat_pong') {
        if (!authenticated) return refuse();
        state.heartbeats++;
      }
    });
  });
  await new Promise((resolve, reject) => { server.once('error', reject); server.listen(0, host, resolve); });
  return {port: server.address().port, token, state, close: async () => {
    for (const socket of wss.clients) socket.terminate();
    await new Promise(resolve => wss.close(resolve));
    await new Promise(resolve => server.close(resolve));
  }};
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const [certificate, key] = process.argv.slice(2);
  const fixture = await startFixture({certificate: fs.readFileSync(certificate), key: fs.readFileSync(key), host: '0.0.0.0'});
  // Private IPC pipe to the runner, never emitted in hosted logs or artifacts.
  process.stdout.write(JSON.stringify({port: fixture.port, registration_token: fixture.token}) + '\n');
  for (const signal of ['SIGTERM', 'SIGINT']) process.on(signal, () => { void fixture.close().then(() => process.exit(0)); });
}
