import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import {randomUUID} from 'node:crypto';
import {test} from 'node:test';
import vm from 'node:vm';

const source = name => readFileSync(new URL(`../resources/machine-browser/${name}.js`, import.meta.url), 'utf8');
const origin = 'https://signin.example';
const request = (field = 'password') => ({nonce: randomUUID(), field, allowed_origins: [origin], expires_at_ms: Date.now() + 20000});

function fixture() {
  let receive;
  const listeners = new Map(), observers = [];
  class Input {
    type = 'password'; isConnected = true; disabled = false; readOnly = false; value = '';
    getClientRects() { return [{}]; }
    select() {}
  }
  const input = new Input();
  const document = {
    activeElement: input, visibilityState: 'visible', hasFocus: () => true,
    addEventListener: (name, callback) => listeners.set(name, callback),
    execCommand: (_command, _ui, value) => { input.value = value; return true; },
  };
  const context = vm.createContext({
    URL, TextEncoder, Date, crypto: {randomUUID}, HTMLInputElement: Input,
    location: {origin}, document,
    window: {addEventListener() {}, removeEventListener() {}},
    MutationObserver: class { constructor(callback) { observers.push(callback); } observe() {} disconnect() {} },
    chrome: {runtime: {id: 'supervised', onMessage: {addListener(callback) { receive = callback; }}}},
  });
  vm.runInContext(source('policy') + '\n' + source('content'), context);
  return {
    input, document, context, listeners, observers,
    call(message, sender = 'supervised') {
      let response;
      receive(message, {id: sender}, value => { response = JSON.parse(JSON.stringify(value)); });
      return response;
    },
  };
}

test('policy requires exact HTTPS origins, bounded values and suitable input kinds', () => {
  const context = vm.createContext({URL, TextEncoder, Date});
  const policy = vm.runInContext(source('policy') + '\nNyxIdFillerPolicy', context);
  assert.equal(policy.validate(request()), null);
  for (const bad of ['http://signin.example', origin + '/', origin + '/login', 'https://user:pass@signin.example']) {
    assert.equal(policy.validate({...request(), allowed_origins: [bad]}), 'origin_mismatch');
  }
  assert.equal(policy.validate({...request(), expires_at_ms: Date.now() - 1}), 'expired');
  assert.equal(policy.validate({...request(), nonce: 'unbound'}), 'invalid_nonce');
  for (const value of ['', 'a\nb', 'a\0b', 'x'.repeat(16385)]) assert.equal(policy.valueAllowed(value), false);
  for (const type of ['text', 'email', 'tel', 'password', 'number', 'hidden', 'file', 'checkbox']) {
    assert.equal(policy.suitable('password', type), type === 'password');
    assert.equal(policy.suitable('username', type), ['text', 'email', 'tel'].includes(type));
    assert.equal(policy.suitable('one_time_code', type), ['text', 'number', 'tel'].includes(type));
  }
});

test('isolated content refuses foreign origins, hidden/wrong fields and foreign senders without typing', () => {
  const f = fixture(), base = request();
  assert.equal(f.call({...base, operation: 'probe'}, 'other-extension'), undefined);
  assert.equal(f.call({...base, allowed_origins: ['https://other.example']}).reason, 'origin_mismatch');
  assert.equal(f.call({...base, allowed_origins: ['https://*.example']}).reason, 'origin_mismatch');
  f.input.type = 'text';
  assert.equal(f.call({...base, operation: 'probe'}).reason, 'wrong_field');
  f.input.type = 'password'; f.document.visibilityState = 'hidden';
  assert.equal(f.call({...base, operation: 'probe'}).reason, 'not_focused');
  assert.equal(f.input.value, '');
});

test('one probed field consumes one nonce, checks focus again and pins passwords against reveal and copy', () => {
  const f = fixture(), base = request(), secret = randomUUID();
  const ready = f.call({...base, operation: 'probe'});
  assert.equal(ready.status, 'ready');
  const filled = f.call({...base, operation: 'fill', token: ready.token, value: secret});
  assert.deepEqual(filled, {status: 'filled', field: 'password', origin});
  assert.equal(f.input.value, secret);
  assert.equal(JSON.stringify(filled).includes(secret), false);
  assert.equal(f.call({...base, operation: 'fill', token: ready.token, value: secret}).reason, 'replayed');
  f.input.type = 'text'; f.observers[0]();
  assert.equal(f.input.type, 'password');
  for (const kind of ['copy', 'cut']) {
    let prevented = false;
    f.listeners.get(kind)({composedPath: () => [f.input], preventDefault() { prevented = true; }, stopImmediatePropagation() {}});
    assert.equal(prevented, true);
  }
  const next = request(), probe = f.call({...next, operation: 'probe'});
  f.document.activeElement = new f.input.constructor();
  assert.equal(f.call({...next, operation: 'fill', token: probe.token, value: secret}).reason, 'focus_changed');
  assert.equal(f.document.activeElement.value, '');
});

test('signed CRX is fresh and its package pin matches the deterministic sources', async () => {
  const {createHash, createPublicKey, verify} = await import('node:crypto');
  const {spawnSync} = await import('node:child_process');
  const {fileURLToPath} = await import('node:url');
  const root = fileURLToPath(new URL('../resources/machine-browser/', import.meta.url));
  const built = spawnSync('python3', ['-c', `import io,sys,zipfile,pathlib
out=io.BytesIO()
with zipfile.ZipFile(out,'w',zipfile.ZIP_DEFLATED) as z:
 for p in sorted(pathlib.Path(sys.argv[1]).glob('*.js'))+[pathlib.Path(sys.argv[1])/'manifest.json']:
  i=zipfile.ZipInfo(p.name,(2026,1,1,0,0,0));i.compress_type=zipfile.ZIP_DEFLATED;i.external_attr=0o100644<<16;z.writestr(i,p.read_bytes())
sys.stdout.buffer.write(out.getvalue())`, root]);
  assert.equal(built.status, 0);
  const crx = readFileSync(new URL('../resources/machine-browser/filler.crx', import.meta.url));
  assert.equal(crx.subarray(0,4).toString(), 'Cr24');
  assert.equal(crx.readUInt32LE(4), 3);
  const headerEnd = 12 + crx.readUInt32LE(8);
  assert.deepEqual(crx.subarray(headerEnd), built.stdout, 'Extension sources changed: run node cli/scripts/package-machine-filler.mjs');
  function fields(bytes) {
    let at=0;
    const variable=()=>{let value=0,shift=0,b;do{assert(at<bytes.length);b=bytes[at++];value+=(b&127)*2**shift;shift+=7;assert(shift<=35);}while(b&128);return value;};
    const result=new Map();
    while(at<bytes.length){const tag=variable();assert.equal(tag&7,2);const size=variable();assert(at+size<=bytes.length);result.set(tag>>>3,bytes.subarray(at,at+size));at+=size;}
    return result;
  }
  const header=fields(crx.subarray(12,headerEnd)), proof=fields(header.get(2)), signed=header.get(10000);
  const key=proof.get(1), digest=createHash('sha256').update(key).digest().subarray(0,16);
  assert.deepEqual(fields(signed).get(1),digest);
  const length=Buffer.alloc(4);length.writeUInt32LE(signed.length);
  assert(verify('sha256',Buffer.concat([Buffer.from('CRX3 SignedData\0'),length,signed,built.stdout]),createPublicKey({key,format:'der',type:'spki'}),proof.get(2)));
  const pin=JSON.parse(readFileSync(new URL('../resources/machine-browser/package.json',import.meta.url)));
  assert.equal(pin.sha256,createHash('sha256').update(crx).digest('hex'));
  assert.equal(pin.extension_id,digest.toString('hex').replace(/[0-9a-f]/g,c=>String.fromCharCode(97+parseInt(c,16))));
  assert.equal(pin.version,JSON.parse(readFileSync(new URL('../resources/machine-browser/manifest.json',import.meta.url))).version);
});
