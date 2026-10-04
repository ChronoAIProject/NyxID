import { createServer } from 'node:http';
import { readFile } from 'node:fs/promises';
import { timingSafeEqual } from 'node:crypto';
import { pathToFileURL } from 'node:url';
import mqtt from 'mqtt';

const MAX_BODY_BYTES = 4096;
const ID = /^[a-z][a-z0-9_-]{0,63}$/;
export const ENTITY = /^(light|switch|fan|climate)\.[a-z0-9_]+$/;

export function validateConfig(config) {
  if (!config || typeof config !== 'object' || !Array.isArray(config.devices) || !config.devices.length) {
    throw new Error('Config must contain a nonempty devices array');
  }
  if (config.homeAssistant && (typeof config.homeAssistant.url !== 'string' || !/^https?:\/\//.test(config.homeAssistant.url))) {
    throw new Error('homeAssistant.url must be an HTTP URL');
  }
  if (config.mqtt && (typeof config.mqtt.url !== 'string' || !/^mqtts?:\/\//.test(config.mqtt.url))) {
    throw new Error('mqtt.url must be an MQTT URL');
  }
  for (const source of [config.homeAssistant, config.mqtt]) {
    if (!source) continue;
    let url;
    try { url = new URL(source.url); }
    catch { throw new Error('Connection URL is invalid'); }
    if (!url.hostname || url.username || url.password || url.search || url.hash) {
      throw new Error('Connection URLs must not contain credentials, queries, or fragments');
    }
  }
  const ids = new Set();
  const stateTopics = new Set();
  for (const device of config.devices) {
    if (!ID.test(device.id || '') || ids.has(device.id) || typeof device.name !== 'string' || !device.name.trim()) {
      throw new Error('Each device needs a unique id and nonempty name');
    }
    ids.add(device.id);
    if (!['home_assistant', 'mqtt'].includes(device.source) || !config[device.source === 'mqtt' ? 'mqtt' : 'homeAssistant']) {
      throw new Error(`Device ${device.id} has an unconfigured source`);
    }
    if (device.source === 'home_assistant' && !ENTITY.test(device.entityId || '')) {
      throw new Error(`Device ${device.id} has an invalid or unsupported entityId`);
    }
    if (device.source === 'mqtt' && (typeof device.stateTopic !== 'string' || !device.stateTopic || /[#+]/.test(device.stateTopic))) {
      throw new Error(`Device ${device.id} needs an exact stateTopic`);
    }
    if (device.source === 'mqtt') {
      if (stateTopics.has(device.stateTopic)) throw new Error(`Duplicate MQTT stateTopic: ${device.stateTopic}`);
      stateTopics.add(device.stateTopic);
    }
    if (device.actions !== undefined && (!Array.isArray(device.actions) || device.actions.some(a => !['turn_on', 'turn_off'].includes(a)) || new Set(device.actions).size !== device.actions.length)) {
      throw new Error(`Device ${device.id} has unsupported actions`);
    }
    if (device.actions?.length && device.source === 'mqtt') {
      if (typeof device.commandTopic !== 'string' || !device.commandTopic || /[#+]/.test(device.commandTopic)
          || typeof device.onPayload !== 'string' || typeof device.offPayload !== 'string'
          || device.onPayload.length > 1024 || device.offPayload.length > 1024) {
        throw new Error(`Device ${device.id} needs exact commandTopic and bounded on/off payloads`);
      }
    }
  }
  return config;
}

function safeEqual(a, b) {
  const left = Buffer.from(a);
  const right = Buffer.from(b);
  return left.length === right.length && timingSafeEqual(left, right);
}

function send(res, status, body) {
  const json = JSON.stringify(body);
  res.writeHead(status, { 'Content-Type': 'application/json', 'Content-Length': Buffer.byteLength(json), 'Cache-Control': 'no-store' });
  res.end(json);
}

async function readJson(req) {
  let size = 0;
  const chunks = [];
  for await (const chunk of req) {
    size += chunk.length;
    if (size > MAX_BODY_BYTES) {
      const error = new Error('Request body too large');
      error.status = 413;
      throw error;
    }
    chunks.push(chunk);
  }
  try { return JSON.parse(Buffer.concat(chunks).toString('utf8')); }
  catch { const error = new Error('Invalid JSON'); error.status = 400; throw error; }
}

export function createBridge(config, key, adapters) {
  validateConfig(config);
  if (typeof key !== 'string' || key.length < 32) throw new Error('BRIDGE_KEY must be at least 32 characters');
  const devices = new Map(config.devices.map(d => [d.id, d]));
  return createServer(async (req, res) => {
    if (!safeEqual(req.headers.authorization || '', `Bearer ${key}`)) {
      send(res, 401, { error: 'Unauthorized' });
      return;
    }
    try {
      const path = new URL(req.url, 'http://localhost').pathname;
      if (req.method === 'GET' && path === '/devices') {
        const result = await Promise.all(config.devices.map(async d => {
          try { return await adapters[d.source].state(d); }
          catch { return { id: d.id, name: d.name, source: d.source, state: null, actions: d.actions || [], unavailable: true }; }
        }));
        send(res, 200, { devices: result });
        return;
      }
      const match = /^\/devices\/([a-z][a-z0-9_-]{0,63})(?:\/actions)?$/.exec(path);
      const device = match && devices.get(match[1]);
      if (!device) { send(res, 404, { error: 'Device not found' }); return; }
      if (req.method === 'GET' && path === `/devices/${device.id}`) {
        send(res, 200, await adapters[device.source].state(device));
        return;
      }
      if (req.method === 'POST' && path === `/devices/${device.id}/actions`) {
        const body = await readJson(req);
        if (!body || Object.keys(body).length !== 1 || !['turn_on', 'turn_off'].includes(body.action)
            || !device.actions?.includes(body.action)) {
          send(res, 403, { error: 'Action is not allowed for this device' });
          return;
        }
        await adapters[device.source].act(device, body.action);
        send(res, 200, { accepted: true, device_id: device.id, action: body.action });
        return;
      }
      send(res, 405, { error: 'Method not allowed' });
    } catch (error) {
      if (error.status) send(res, error.status, { error: error.message });
      else { console.error('Device operation failed:', error.message); send(res, 502, { error: 'Device operation failed' }); }
    }
  });
}

export function homeAssistantAdapter(config, token, fetchImpl = fetch) {
  if (!token) throw new Error('HA_TOKEN is required');
  const base = new URL(config.url);
  if (base.username || base.password || base.search || base.hash) throw new Error('Home Assistant URL must not contain credentials or a query');
  base.pathname = base.pathname.replace(/\/?$/, '/');
  async function call(path, init = {}) {
    const response = await fetchImpl(new URL(path, base), {
      ...init, redirect: 'error', signal: AbortSignal.timeout(10000),
      headers: { Authorization: `Bearer ${token}`, 'Content-Type': 'application/json' },
    });
    if (!response.ok) throw new Error(`Home Assistant returned ${response.status}`);
    return response.status === 204 ? null : response.json();
  }
  return {
    async state(device) {
      const value = await call(`api/states/${encodeURIComponent(device.entityId)}`);
      return { id: device.id, name: device.name, source: device.source, state: value.state, actions: device.actions || [] };
    },
    async act(device, action) {
      const domain = device.entityId.split('.')[0];
      await call(`api/services/${domain}/${action}`, { method: 'POST', body: JSON.stringify({ entity_id: device.entityId }) });
    },
  };
}

export async function mqttAdapter(config, devices, options = {}) {
  const client = (options.connect || mqtt.connect)(config.url, {
    username: options.username, password: options.password,
    reconnectPeriod: 2000, connectTimeout: 10000, clean: true,
  });
  const states = new Map();
  const topics = new Map(devices.filter(d => d.source === 'mqtt').map(d => [d.stateTopic, d.id]));
  client.on('connect', () => client.subscribe([...topics.keys()], { qos: 0 }, error => {
    if (error) console.error('MQTT subscription failed:', error.message);
  }));
  for (const event of ['offline', 'close']) client.on(event, () => states.clear());
  client.on('error', error => console.error('MQTT connection failed:', error.message));
  client.on('message', (topic, payload) => {
    const id = topics.get(topic);
    if (id && payload.length <= 1024) states.set(id, payload.toString('utf8'));
  });
  return {
    state(device) {
      return { id: device.id, name: device.name, source: device.source, state: client.connected ? states.get(device.id) ?? null : null, actions: device.actions || [] };
    },
    act(device, action) {
      if (!client.connected) throw new Error('MQTT broker is offline');
      const payload = action === 'turn_on' ? device.onPayload : device.offPayload;
      return new Promise((resolve, reject) => client.publish(device.commandTopic, payload, { qos: 1, retain: false }, error => error ? reject(error) : resolve()));
    },
    close() { client.end(true); },
  };
}

async function main() {
  const config = validateConfig(JSON.parse(await readFile(process.env.BRIDGE_CONFIG || './config.json', 'utf8')));
  const key = process.env.BRIDGE_KEY || (await readFile(process.env.BRIDGE_KEY_FILE || './bridge.key', 'utf8')).trim();
  const adapters = {};
  if (config.homeAssistant) adapters.home_assistant = homeAssistantAdapter(config.homeAssistant, process.env.HA_TOKEN);
  if (config.mqtt) adapters.mqtt = await mqttAdapter(config.mqtt, config.devices, {
    username: process.env.MQTT_USERNAME, password: process.env.MQTT_PASSWORD,
  });
  const server = createBridge(config, key, adapters);
  const port = Number(process.env.BRIDGE_PORT || 8787);
  if (!Number.isInteger(port) || port < 1 || port > 65535) throw new Error('Invalid BRIDGE_PORT');
  server.listen(port, '127.0.0.1', () => console.info(`Home automation bridge listening on 127.0.0.1:${port}`));
  for (const signal of ['SIGINT', 'SIGTERM']) process.on(signal, () => {
    adapters.mqtt?.close();
    server.close();
  });
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  main().catch(error => { console.error(error.message); process.exitCode = 1; });
}
