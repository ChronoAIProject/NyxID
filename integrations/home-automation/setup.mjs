import { randomBytes } from 'node:crypto';
import { access, readFile, writeFile } from 'node:fs/promises';
import { createInterface } from 'node:readline/promises';
import { pathToFileURL } from 'node:url';
import { ENTITY, validateConfig } from './bridge.mjs';

function connectionUrl(value, protocols) {
  let url;
  try { url = new URL(value); }
  catch { throw new Error('Connection URL is invalid'); }
  if (!protocols.includes(url.protocol) || !url.hostname || url.username || url.password || url.search || url.hash) {
    throw new Error('Connection URL must use the expected protocol and contain no credentials, query, or fragment');
  }
  return url;
}

export async function discoverHomeAssistant(rawUrl, token, fetchImpl = fetch) {
  if (!token) throw new Error('Set HA_TOKEN before running setup');
  const url = connectionUrl(rawUrl, ['http:', 'https:']);
  url.pathname = `${url.pathname.replace(/\/?$/, '/')}api/states`;
  const response = await fetchImpl(url, {
    redirect: 'error',
    signal: AbortSignal.timeout(10000),
    headers: { Authorization: `Bearer ${token}` },
  });
  if (!response.ok) throw new Error(`Home Assistant state discovery failed (${response.status})`);
  const states = await response.json();
  if (!Array.isArray(states)) throw new Error('Home Assistant returned an invalid state list');
  return states
    .filter(row => row && ENTITY.test(row.entity_id || ''))
    .map(row => ({ entityId: row.entity_id, name: String(row.attributes?.friendly_name || row.entity_id).replace(/[\x00-\x1f\x7f]/g, ' ').slice(0, 100) }))
    .sort((a, b) => a.entityId.localeCompare(b.entityId));
}

export function parseSelection(input, count, allowEmpty = false) {
  const trimmed = input.trim();
  if (!trimmed && allowEmpty) return [];
  if (!/^\d+(\s*,\s*\d+)*$/.test(trimmed)) throw new Error('Enter comma-separated numbers from the list');
  const indexes = trimmed.split(',').map(value => Number(value.trim()) - 1);
  if (indexes.some(index => index < 0 || index >= count) || new Set(indexes).size !== indexes.length) {
    throw new Error('Selection contains an unknown or repeated number');
  }
  return indexes;
}

export function mapHomeAssistantDevices(entities, selected, writable = []) {
  const writableSet = new Set(writable);
  const ids = new Set();
  return selected.map((entityIndex, selectedIndex) => {
    const entity = entities[entityIndex];
    if (!entity) throw new Error('Selected entity is unavailable');
    const prefix = entity.entityId.replace('.', '_').slice(0, 60);
    let id = prefix;
    for (let suffix = 2; ids.has(id); suffix++) id = `${prefix}_${suffix}`;
    ids.add(id);
    return {
      id, name: entity.name, source: 'home_assistant', entityId: entity.entityId,
      actions: writableSet.has(selectedIndex) ? ['turn_on', 'turn_off'] : [],
    };
  });
}

async function askRequired(ui, prompt) {
  const value = (await ui.question(prompt)).trim();
  if (!value) throw new Error('A value is required');
  return value;
}

async function fileExists(path) {
  try { await access(path); return true; }
  catch (error) { if (error.code === 'ENOENT') return false; throw error; }
}

async function main() {
  const configPath = process.env.BRIDGE_CONFIG || './config.json';
  const keyPath = process.env.BRIDGE_KEY_FILE || './bridge.key';
  if (await fileExists(configPath)) throw new Error(`${configPath} already exists; edit it directly or choose another BRIDGE_CONFIG path`);
  const ui = createInterface({ input: process.stdin, output: process.stdout });
  try {
    const choice = (await askRequired(ui, 'Connect Home Assistant, MQTT, or both? [ha/mqtt/both]: ')).toLowerCase();
    if (!['ha', 'mqtt', 'both'].includes(choice)) throw new Error('Choose ha, mqtt, or both');
    const config = { devices: [] };
    if (choice !== 'mqtt') {
      const url = await askRequired(ui, 'Home Assistant URL: ');
      const entities = await discoverHomeAssistant(url, process.env.HA_TOKEN);
      if (!entities.length) throw new Error('No supported Home Assistant entities found');
      console.info('\nAvailable entities:');
      entities.forEach((entity, index) => console.info(`${index + 1}. ${entity.name} (${entity.entityId})`));
      const selected = parseSelection(await askRequired(ui, 'Devices to connect (numbers, comma-separated): '), entities.length);
      const selectedEntities = selected.map(index => entities[index]);
      console.info('\nSelected devices:');
      selectedEntities.forEach((entity, index) => console.info(`${index + 1}. ${entity.name} (${entity.entityId})`));
      const writable = parseSelection(await ui.question('Allow on/off for which selected devices? [blank = read-only]: '), selected.length, true);
      config.homeAssistant = { url };
      config.devices.push(...mapHomeAssistantDevices(entities, selected, writable));
    }
    if (choice !== 'ha') {
      config.mqtt = { url: await askRequired(ui, 'MQTT broker URL (mqtt:// or mqtts://): ') };
      const count = Number(await askRequired(ui, 'Number of MQTT devices to map: '));
      if (!Number.isInteger(count) || count < 1 || count > 100) throw new Error('Map between 1 and 100 MQTT devices');
      for (let index = 0; index < count; index++) {
        console.info(`\nMQTT device ${index + 1}:`);
        const device = {
          id: await askRequired(ui, 'ID (lowercase letters, numbers, underscores): '),
          name: await askRequired(ui, 'Name: '),
          source: 'mqtt',
          stateTopic: await askRequired(ui, 'Exact state topic: '),
          actions: [],
        };
        const commandTopic = (await ui.question('Exact command topic [blank = read-only]: ')).trim();
        if (commandTopic) {
          device.commandTopic = commandTopic;
          device.onPayload = await askRequired(ui, 'On payload: ');
          device.offPayload = await askRequired(ui, 'Off payload: ');
          device.actions = ['turn_on', 'turn_off'];
        }
        config.devices.push(device);
      }
    }
    validateConfig(config);
    let key;
    if (await fileExists(keyPath)) {
      key = (await readFile(keyPath, 'utf8')).trim();
      if (key.length < 32) throw new Error(`${keyPath} contains a bridge key shorter than 32 characters`);
    } else {
      key = randomBytes(32).toString('hex');
      await writeFile(keyPath, `${key}\n`, { mode: 0o600, flag: 'wx' });
    }
    await writeFile(configPath, `${JSON.stringify(config, null, 2)}\n`, { mode: 0o600, flag: 'wx' });
    console.info(`\nSaved ${config.devices.length} devices to ${configPath}. Bridge key is in ${keyPath}.`);
    console.info('Use that bridge key when setting up this service\'s credentials on the NyxID node.');
  } finally { ui.close(); }
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  main().catch(error => { console.error(error.message); process.exitCode = 1; });
}
