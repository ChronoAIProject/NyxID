// Used by native relay QA with a local production backend on port 3011 and
// its own temporary database. Credentials and generated frames stay in memory.
import assert from 'node:assert/strict';
import { randomBytes, randomUUID } from 'node:crypto';
import { createRequire } from 'node:module';
import { RED_JPEG, GREEN_PATCH, BLUE_JPEG, vector } from '../testVectors.ts';

const require = createRequire(new URL('../../../../../package.json', import.meta.url));
const WebSocket = require('ws');
const base = 'http://127.0.0.1:3011/api/v1';

export async function relayFixture() {
  let accessToken;
  let node;
  let closed = false;
  const timers = new Set();
  const sessions = new Map();
  const controllers = new Map();
  const metrics = { opens: 0, refreshes: 0, fullFrames: 0, patchFrames: 0, controlChanges: 0, handBacks: 0, inputs: 0, inputTools: [], stops: 0, forbiddenOperations: [] };
  const api = async (path, body) => {
    const response = await fetch(base + path, {
      method: body ? 'POST' : 'GET',
      headers: { 'Content-Type': 'application/json', ...(accessToken ? { Authorization: `Bearer ${accessToken}` } : {}) },
      ...(body ? { body: JSON.stringify(body) } : {}),
      signal: AbortSignal.timeout(15000),
    });
    if (!response.ok) throw new Error(`Local relay fixture: ${path} returned ${response.status}`);
    return response.json();
  };
  const later = (action, delay) => {
    const timer = setTimeout(() => { timers.delete(timer); if (!closed) action(); }, delay);
    timers.add(timer);
  };
  const frame = (jpeg, id, sequence, rectangle) => {
    const bytes = new Uint8Array(vector(new Uint8Array(Buffer.from(jpeg, 'base64')), sequence, rectangle));
    bytes.set(Buffer.from(id.replaceAll('-', ''), 'hex'), 6);
    return bytes;
  };
  const sendFrames = id => {
    const first = (sessions.get(id) ?? 0n) + 1n;
    sessions.set(id, first + 2n);
    const frames = [
      frame(RED_JPEG, id, first),
      frame(GREEN_PATCH, id, first + 1n, { width: 8, height: 6, x: 2, y: 2, patchWidth: 2, patchHeight: 2, base: first }),
      frame(BLUE_JPEG, id, first + 2n),
    ];
    frames.forEach((bytes, index) => later(() => {
      if (node.readyState !== WebSocket.OPEN) return;
      node.send(bytes, { binary: true });
      if (index === 1) metrics.patchFrames++; else metrics.fullFrames++;
    }, 150 + index * 350));
  };
  const close = () => {
    if (closed) return;
    closed = true;
    for (const timer of timers) clearTimeout(timer);
    timers.clear();
    node?.close();
    accessToken = undefined;
  };
  try {
    const email = `desktop-${randomUUID()}@fixture.invalid`;
    const password = randomBytes(24).toString('base64url');
    await api('/auth/register', { email, password, display_name: 'Synthetic desktop owner' });
    const login = await api('/auth/login', { email, password, client: 'mobile' });
    assert.ok(login.access_token && login.refresh_token && login.user_id, 'Mobile first-party login');
    accessToken = login.access_token;
    const registration = await api('/nodes/register-token', { name: 'Synthetic native desktop source' });
    node = new WebSocket('ws://127.0.0.1:3011/api/v1/nodes/ws');
    await new Promise((resolve, reject) => { node.once('open', resolve); node.once('error', reject); });
    const profile = {
      version: 1, runtime_id: randomUUID(), computer: true, computer_ready: true,
      computer_tools: ['get_window_state'], browser_tools: true, os: 'synthetic', arch: 'qa',
    };
    const nodeId = await new Promise((resolve, reject) => {
      const timeout = setTimeout(() => reject(new Error('Local node registration deadline')), 15000);
      timers.add(timeout);
      const receive = bytes => {
        const value = JSON.parse(bytes.toString());
        if (value.type !== 'register_ok') { reject(new Error('Local node registration refused')); return; }
        clearTimeout(timeout); timers.delete(timeout); node.off('message', receive);
        // auth_token/signing_secret from registration are deliberately discarded.
        node.send(JSON.stringify({ type: 'status_update', agent_version: 'native-desktop-qa', capabilities: { machine: profile } }));
        resolve(value.node_id);
      };
      node.on('message', receive);
      node.send(JSON.stringify({ type: 'register', token: registration.token, metadata: { os: 'synthetic', arch: 'qa' } }));
    });
    node.on('message', (bytes, binary) => {
      if (typeof binary === 'boolean' ? binary : typeof bytes !== 'string') {
        try {
          const frame = new Uint8Array(bytes);
          assert.equal(Buffer.from(frame.slice(0,6)).toString('hex'),'4e59584d0200');
          const request = JSON.parse(Buffer.from(frame.slice(30)).toString('utf8'));
          const control = controllers.get(request.parameters.session_id);
          assert.equal(request.operation,'desktop_input');
          assert.ok(control?.owner && request.parameters.viewer_id === control.viewer && request.parameters.revision === control.revision,'server injects current owner/revision fence');
          assert.ok(['click','drag','scroll','press_key','type_text'].includes(request.parameters.tool));
          metrics.inputs++;metrics.inputTools.push(request.parameters.tool);
        } catch { metrics.forbiddenOperations.push('invalid_binary_input'); }
        return;
      }
      const value = JSON.parse(bytes.toString());
      if (value.type === 'heartbeat_ping') {
        node.send(JSON.stringify({ type: 'heartbeat_pong', timestamp: value.timestamp }));
        return;
      }
      if (value.type !== 'machine_request') return;
      if (value.operation === 'desktop_control') {
        const args = value.parameters;
        assert.equal(typeof args.viewer_id,'string');assert.equal(typeof args.revision,'number');
        const previous = controllers.get(args.session_id);
        assert.ok(!previous || args.revision >= previous.revision,'control revisions never go backwards');
        controllers.set(args.session_id,{owner:args.owner,viewer:args.viewer_id,revision:args.revision});
        metrics.controlChanges++;if (!args.owner) metrics.handBacks++;
        node.send(JSON.stringify({type:'machine_result',request_id:value.request_id,result:{ok:true}}));return;
      }
      if (value.operation === 'cancel') {
        metrics.stops++;node.send(JSON.stringify({type:'machine_result',request_id:value.request_id,result:{ok:true}}));return;
      }
      if (value.operation !== 'desktop_open') {
        metrics.forbiddenOperations.push(value.operation);
        node.send(JSON.stringify({ type: 'machine_result', request_id: value.request_id,
          result: { error: { code: 12416, message: 'Synthetic source received an unsupported command' } } }));
        return;
      }
      metrics.opens++;
      const id = value.parameters.session_id;
      assert.match(id, /^[\da-f]{8}(-[\da-f]{4}){3}-[\da-f]{12}$/i);
      node.send(JSON.stringify({ type: 'machine_result', request_id: value.request_id, result: { ok: true } }));
      if (!sessions.has(id) || value.parameters.refresh_frame) {
        if (sessions.has(id)) metrics.refreshes++;
        sendFrames(id);
      }
    });
    // The register response precedes the durable owner claim/profile update.
    let ready = false;
    for (let attempt = 0; attempt < 30; attempt++) {
      try { await api(`/assistant/nyxagent/machines/${nodeId}/contexts`); ready = true; break; } catch {
        await new Promise(resolve => setTimeout(resolve, 200));
      }
    }
    assert.ok(ready, 'Production backend recognizes the connected desktop source');
    return {
      params: { nodeId, display: 'secure' }, metrics, close,
      session: { accessToken, refreshToken: login.refresh_token, userId: login.user_id,
        accessTokenExpiresAt: Date.now() + login.expires_in * 1000 },
    };
  } catch (error) { close(); throw error; }
}
