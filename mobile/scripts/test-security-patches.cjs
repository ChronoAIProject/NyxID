// Run after the frozen install. NYXID_SECURITY_PACKAGE_ROOT permits the same
// regressions to demonstrate failure against authenticated original tarballs.
const assert = require('node:assert/strict');
const crypto = require('node:crypto');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const { spawnSync } = require('node:child_process');
const { test } = require('node:test');
const root = process.env.NYXID_SECURITY_PACKAGE_ROOT || path.resolve(__dirname, '../node_modules');
const load = name => require(path.join(root, name));

function imageBox(name, length = 8) {
  const box = Buffer.alloc(8);
  box.writeUInt32BE(length); box.write(name, 4);
  return box;
}

function icnsEntry(name, length = 8) {
  const entry = Buffer.alloc(8);
  entry.write(name); entry.writeUInt32BE(length, 4);
  return entry;
}

function probe(name) {
  if (name === 'uri') {
    const decode = load('decode-uri-component');
    const input = '%C3%A5%FF'.repeat(4000);
    assert.equal(decode(input), 'å%FF'.repeat(4000));
  } else if (name === 'uri_distinct') {
    const decode = load('decode-uri-component');
    const values = Array.from({ length: 16000 }, (_, index) => String(index).padStart(6, '0'));
    const input = values.map(value => '%FF' + [...value].map(char => '%' + char.charCodeAt(0).toString(16)).join('')).join('/');
    const started = performance.now();
    assert.equal(decode(input), values.map(value => '%FF' + value).join('/'));
    assert.ok(performance.now() - started < 1500, 'distinct malformed runs must finish within the bounded CPU budget');
  } else if (name === 'image') {
    const imageSize = load('image-size');
    // Actual container signature + ftyp + zero-sized matching jxlp. The old
    // extractor repeatedly selects the same box without advancing its offset.
    const signature = Buffer.from('0000000c4a584c200d0a870a', 'hex');
    const ftyp = Buffer.concat([imageBox('ftyp', 20), Buffer.from('jxl \0\0\0\0jxl ')]);
    assert.throws(() => imageSize(Buffer.concat([signature, ftyp, imageBox('jxlp', 0)])), /Invalid.*(?:box|stream).*size/);
  } else if (name === 'icns') {
    const imageSize = load('image-size');
    for (const length of [0, 1, 7, 0xffffffff]) {
      for (const entries of [[icnsEntry('icp4', length)], [icnsEntry('icp4'), icnsEntry('icp5', length)]]) {
        const header = Buffer.concat([Buffer.from('icns'), Buffer.alloc(4)]);
        header.writeUInt32BE(8 + entries.length * 8, 4);
        assert.throws(() => imageSize(Buffer.concat([header, ...entries])), /Invalid ICNS/);
      }
    }
  } else if (name === 'forge' || name === 'forge_der') {
    const forge = load('node-forge');
    // Retain the low exponent used by the forged-DigestInfo regression, with
    // a production-strength modulus so the fixture does not use a weak key.
    const keys = crypto.generateKeyPairSync('rsa', { modulusLength: 2048, publicExponent: 3 });
    const privateKey = forge.pki.privateKeyFromPem(keys.privateKey.export({ type: 'pkcs1', format: 'pem' }));
    const publicKey = forge.pki.publicKeyFromPem(keys.publicKey.export({ type: 'pkcs1', format: 'pem' }));
    const digest = forge.md.sha256.create().update('bounded fixture').digest().getBytes();
    const { asn1 } = forge;
    const item = (type, value, constructed = false) => asn1.create(asn1.Class.UNIVERSAL, type, constructed, value);
    for (const withNull of [false, true]) {
      const algorithm = [item(asn1.Type.OID, asn1.oidToDer(forge.oids.sha256).getBytes())];
      if (withNull) algorithm.push(item(asn1.Type.NULL, ''));
      const encode = fields => asn1.toDer(item(asn1.Type.SEQUENCE, [
        item(asn1.Type.SEQUENCE, fields, true), item(asn1.Type.OCTETSTRING, digest),
      ], true)).getBytes();
      assert.equal(publicKey.verify(digest, privateKey.sign(encode(algorithm), null)), true);
      const malformed = encode([...algorithm, item(asn1.Type.OCTETSTRING, 'extra nested element')]);
      const signature = privateKey.sign(malformed, null);
      if (name === 'forge') assert.throws(() => publicKey.verify(digest, signature), /valid RSASSA-PKCS1-v1_5 DigestInfo/);
    }
    const oid = item(asn1.Type.OID, asn1.oidToDer(forge.oids.sha256).getBytes());
    const encode = (parameters, value = digest) => asn1.toDer(item(asn1.Type.SEQUENCE, [
      item(asn1.Type.SEQUENCE, [oid, ...parameters], true), item(asn1.Type.OCTETSTRING, value),
    ], true)).getBytes();
    for (const value of ['\0', 'PM synthetic invalid NULL contents', 'x'.repeat(40)]) {
      const der = encode([item(asn1.Type.NULL, value)]);
      assert.throws(() => publicKey.verify(digest, privateKey.sign(der, null)), /DigestInfo/);
    }
    for (const value of ['', digest.slice(1), digest + '\0']) {
      const der = encode([item(asn1.Type.NULL, '')], value);
      assert.throws(() => publicKey.verify(value, privateKey.sign(der, null)), /digest length/);
    }
    const canonical = encode([item(asn1.Type.NULL, '')]);
    assert.ok(canonical.charCodeAt(1) < 128);
    const nonminimalLength = canonical.slice(0, 1) + '\x81' + canonical.slice(1);
    assert.throws(() => publicKey.verify(digest, privateKey.sign(nonminimalLength, null)), /DigestInfo/);
    for (const parameters of [[item(asn1.Type.OCTETSTRING, '')],
      [item(asn1.Type.NULL, [], true)], [item(asn1.Type.NULL, ''), item(asn1.Type.NULL, '')]]) {
      assert.throws(() => publicKey.verify(digest, privateKey.sign(encode(parameters), null)), /DigestInfo/);
    }
    const md = forge.md.sha256.create().update('normal signed message');
    const signature = privateKey.sign(md);
    assert.equal(publicKey.verify(md.digest().getBytes(), signature), true);
    assert.equal(publicKey.verify(forge.md.sha256.create().update('changed').digest().getBytes(), signature), false);
  } else throw new Error('unknown security probe');
}

if (process.argv[2] === '--probe') {
  probe(process.argv[3]);
} else {
  for (const name of ['uri', 'uri_distinct', 'image', 'icns', 'forge', 'forge_der']) {
    test(name + ' security regression completes and rejects the vulnerable case', () => {
      const child = spawnSync(process.execPath, [__filename, '--probe', name], {
        encoding: 'utf8', timeout: 5000,
        env: { ...process.env, NODE_PATH: [path.resolve(__dirname, '../node_modules'),
          path.resolve(__dirname, '../node_modules/image-size/node_modules')].join(path.delimiter) },
      });
      assert.equal(child.error, undefined, child.error?.message);
      assert.equal(child.status, 0, child.stderr);
    });
  }
  test('CommonJS query decoder preserves valid, malformed, BOM and plus behavior', () => {
    const decode = load('decode-uri-component');
    for (const [input, output] of [['hello+world', 'hello world'], ['%E2%82%AC', '€'],
      ['%FFabc%41', '%FFabcA'], ['%FE%FF', '��'], ['%C2', '�'], ['%E0%A4', '%E0%A4'], ['%G0', '%G0'],
      ['%24%26%FF', '$&%FF'], ['%24%60%FF', '$`%FF'], ['%24%27%FF', "$'%FF"],
      ['%25C2%FF', '%C2%FF'], ['%FE%FF%41', '��A']]) {
      assert.equal(decode(input), output);
    }
    assert.throws(() => decode(null), TypeError);
    // Test the actual consumer's CJS loading and callable API, too.
    const query = require('query-string');
    assert.equal(query.parse('value=%C3%A5%FF').value, 'å%FF');
  });
  test('image-size preserves Metro buffer, synchronous path and asynchronous path API', async () => {
    const imageSize = load('image-size');
    const png = Buffer.from('89504e470d0a1a0a0000000d49484452000000020000000308060000009d74661a0000000049454e44ae426082', 'hex');
    const directory = fs.mkdtempSync(path.join(os.tmpdir(), 'nyxid-image-patch-'));
    const file = path.join(directory, 'fixture.png');
    try {
      fs.writeFileSync(file, png);
      for (const result of [imageSize(png), imageSize(file), await new Promise((resolve, reject) =>
        imageSize(file, (error, value) => error ? reject(error) : resolve(value)))]) {
        assert.equal(result.width, 2); assert.equal(result.height, 3); assert.equal(result.type, 'png');
      }
      const icns = Buffer.concat([Buffer.from('icns'), Buffer.alloc(4), icnsEntry('icp4')]);
      icns.writeUInt32BE(16, 4);
      assert.equal(imageSize(icns).width, 16);
    } finally { fs.rmSync(directory, { recursive: true }); }
  });
}
