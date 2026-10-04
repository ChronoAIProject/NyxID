import test from 'node:test';
import assert from 'node:assert/strict';
import { discoverHomeAssistant, mapHomeAssistantDevices, parseSelection } from './setup.mjs';
import { validateConfig } from './bridge.mjs';

test('Home Assistant discovery filters unsupported entities and keeps names local', async () => {
  const entities = await discoverHomeAssistant('http://ha.local:8123', 'test-token', async (url, init) => {
    assert.equal(String(url), 'http://ha.local:8123/api/states');
    assert.equal(init.headers.Authorization, 'Bearer test-token');
    return {
      ok: true,
      json: async () => [
        { entity_id: 'sensor.temperature', attributes: { friendly_name: 'Temperature' } },
        { entity_id: 'switch.fan', attributes: { friendly_name: 'Fan\nOffice' } },
        { entity_id: 'light.office', attributes: { friendly_name: 'Office light' } },
      ],
    };
  });
  assert.deepEqual(entities, [
    { entityId: 'light.office', name: 'Office light' },
    { entityId: 'switch.fan', name: 'Fan Office' },
  ]);
});

test('selections are bounded and default to read-only', () => {
  const entities = [
    { entityId: 'light.office', name: 'Office light' },
    { entityId: 'switch.fan', name: 'Fan' },
  ];
  assert.deepEqual(parseSelection('1, 2', 2), [0, 1]);
  assert.deepEqual(parseSelection('', 2, true), []);
  assert.throws(() => parseSelection('1,1', 2), /repeated/);
  assert.throws(() => parseSelection('3', 2), /unknown/);
  const mapped = mapHomeAssistantDevices(entities, [0, 1], [1]);
  assert.deepEqual(mapped.map(device => device.actions), [[], ['turn_on', 'turn_off']]);
  assert.deepEqual(mapped.map(device => device.id), ['light_office', 'switch_fan']);
  validateConfig({ homeAssistant: { url: 'http://ha.local:8123' }, devices: mapped });
});

test('Home Assistant discovery rejects embedded credentials and failed authentication', async () => {
  await assert.rejects(discoverHomeAssistant('http://user:pass@ha.local', 'token'), /no credentials/);
  await assert.rejects(discoverHomeAssistant('http://ha.local', 'token', async () => ({ ok: false, status: 401 })), /401/);
});

test('Home Assistant discovery preserves reverse-proxy base paths', async () => {
  await discoverHomeAssistant('https://ha.example/home', 'token', async url => {
    assert.equal(String(url), 'https://ha.example/home/api/states');
    return { ok: true, json: async () => [] };
  });
});
