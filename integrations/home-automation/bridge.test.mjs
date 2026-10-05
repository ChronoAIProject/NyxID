import test from 'node:test';
import assert from 'node:assert/strict';
import { EventEmitter } from 'node:events';
import { createBridge, homeAssistantAdapter, mqttAdapter, validateConfig } from './bridge.mjs';

const key = 'test-bridge-key-with-at-least-32-chars';
const config = {
  homeAssistant: { url: 'http://localhost:8123/' },
  mqtt: { url: 'mqtt://localhost:1883' },
  devices: [
    { id: 'lamp', name: 'Lamp', source: 'home_assistant', entityId: 'light.lamp', actions: ['turn_on'] },
    { id: 'sensor', name: 'Sensor', source: 'mqtt', stateTopic: 'house/sensor', actions: [] },
  ],
};

test('configuration rejects duplicate ids and wildcard MQTT topics', () => {
  assert.throws(() => validateConfig({ ...config, devices: [config.devices[0], config.devices[0]] }), /unique id/);
  assert.throws(() => validateConfig({ ...config, devices: [{ ...config.devices[1], stateTopic: 'house/#' }] }), /exact stateTopic/);
  assert.throws(() => validateConfig({ ...config, devices: [config.devices[1], { ...config.devices[1], id: 'other' }] }), /Duplicate MQTT stateTopic/);
  assert.throws(() => validateConfig({ ...config, devices: [{ ...config.devices[0], entityId: 'cover.garage' }] }), /unsupported entityId/);
  assert.throws(() => validateConfig({ ...config, mqtt: { url: 'mqtt://user:password@localhost:1883' } }), /must not contain credentials/);
});

test('bridge authorizes requests and enforces mapped actions', async () => {
  const calls = [];
  const adapters = {
    home_assistant: {
      state: async d => ({ id: d.id, state: 'off' }),
      act: async (d, action) => calls.push([d.id, action]),
    },
    mqtt: { state: async d => ({ id: d.id, state: null }), act: async () => assert.fail('read-only device') },
  };
  const server = createBridge(config, key, adapters);
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
  const url = `http://127.0.0.1:${server.address().port}`;
  try {
    assert.equal((await fetch(`${url}/devices`)).status, 401);
    const headers = { Authorization: `Bearer ${key}`, 'Content-Type': 'application/json' };
    const list = await (await fetch(`${url}/devices`, { headers })).json();
    assert.equal(list.devices.length, 2);
    assert.equal((await fetch(`${url}/devices/unknown`, { headers })).status, 404);
    assert.equal((await fetch(`${url}/devices/lamp/actions`, { method: 'POST', headers, body: JSON.stringify({ action: 'turn_off' }) })).status, 403);
    assert.equal((await fetch(`${url}/devices/sensor/actions`, { method: 'POST', headers, body: JSON.stringify({ action: 'turn_on' }) })).status, 403);
    assert.equal((await fetch(`${url}/devices/lamp/actions`, { method: 'POST', headers, body: JSON.stringify({ action: 'turn_on' }) })).status, 200);
    assert.deepEqual(calls, [['lamp', 'turn_on']]);
  } finally { await new Promise(resolve => server.close(resolve)); }
});

test('device list keeps working when one integration is unavailable', async () => {
  const server = createBridge(config, key, {
    home_assistant: { state: async () => { throw new Error('offline'); } },
    mqtt: { state: async d => ({ id: d.id, state: 'open' }) },
  });
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
  try {
    const url = `http://127.0.0.1:${server.address().port}`;
    const headers = { Authorization: `Bearer ${key}` };
    const result = await (await fetch(`${url}/devices`, { headers })).json();
    assert.equal(result.devices[0].unavailable, true);
    assert.equal(result.devices[1].state, 'open');
    assert.equal((await fetch(`${url}/devices/lamp`, { headers })).status, 502);
  } finally { await new Promise(resolve => server.close(resolve)); }
});

test('Home Assistant adapter uses only mapped entity and service paths', async () => {
  const calls = [];
  const adapter = homeAssistantAdapter(config.homeAssistant, 'test-token', async (url, init) => {
    calls.push({ url: String(url), init });
    return { ok: true, status: 200, json: async () => ({ state: 'on' }) };
  });
  assert.equal((await adapter.state(config.devices[0])).state, 'on');
  await adapter.act(config.devices[0], 'turn_on');
  assert.equal(calls[0].url, 'http://localhost:8123/api/states/light.lamp');
  assert.equal(calls[1].url, 'http://localhost:8123/api/services/light/turn_on');
  assert.deepEqual(JSON.parse(calls[1].init.body), { entity_id: 'light.lamp' });
  assert.equal(calls[1].init.headers.Authorization, 'Bearer test-token');
});

test('Home Assistant adapter preserves reverse-proxy base paths', async () => {
  const urls = [];
  const adapter = homeAssistantAdapter({ url: 'https://ha.example/home' }, 'test-token', async url => {
    urls.push(String(url));
    return { ok: true, status: 200, json: async () => ({ state: 'on' }) };
  });
  await adapter.state(config.devices[0]);
  await adapter.act(config.devices[0], 'turn_on');
  assert.deepEqual(urls, [
    'https://ha.example/home/api/states/light.lamp',
    'https://ha.example/home/api/services/light/turn_on',
  ]);
});

test('MQTT adapter subscribes exactly and publishes only configured command', async () => {
  const client = new EventEmitter();
  client.connected = true;
  client.subscribe = (topics, options, callback) => { assert.deepEqual(topics, ['house/sensor']); callback(); };
  client.publish = (topic, payload, options, callback) => {
    assert.equal(topic, 'house/lamp/set');
    assert.equal(payload, 'ON');
    assert.deepEqual(options, { qos: 1, retain: false });
    callback();
  };
  client.end = () => {};
  const device = { id: 'sensor', name: 'Sensor', source: 'mqtt', stateTopic: 'house/sensor', commandTopic: 'house/lamp/set', onPayload: 'ON', offPayload: 'OFF', actions: ['turn_on'] };
  const adapter = await mqttAdapter(config.mqtt, [device], { connect: () => client });
  client.emit('connect');
  assert.equal(adapter.state(device).state, null);
  client.emit('message', 'house/sensor', Buffer.from('open'));
  assert.equal(adapter.state(device).state, 'open');
  await adapter.act(device, 'turn_on');
  client.emit('offline');
  client.connected = false;
  assert.equal(adapter.state(device).state, null);
  adapter.close();
});
