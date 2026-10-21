#!/usr/bin/env node
'use strict';

const assert = require('node:assert/strict');
const crypto = require('node:crypto');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const {execFileSync} = require('node:child_process');
const {test} = require('node:test');
const producer = require('./sign-rehearsal.cjs');
const root = process.env.SIGNING_TOOLS_ROOT || path.resolve(__dirname, '../security/signing-tools');
const tools = producer.libraries(root);
const environment = {
  ACTIONS_ID_TOKEN_REQUEST_URL: 'https://fixture.actions.githubusercontent.com/token?api-version=2',
  ACTIONS_ID_TOKEN_REQUEST_TOKEN: 'fixture-job-token',
};

test('reviewed libraries preserve exact W6 DSSE serialization including empty keyid', () => {
  const envelope = {payload: Buffer.from('statement'), payloadType: 'application/vnd.in-toto+json',
    signatures: [{sig: Buffer.from('signature'), keyid: ''}]};
  assert.equal(JSON.stringify(tools.bundle.envelopeToJSON(envelope)),
    '{"payload":"c3RhdGVtZW50","payloadType":"application/vnd.in-toto+json","signatures":[{"sig":"c2lnbmF0dXJl","keyid":""}]}');
});

test('real DSSE signing and Rekor request bind the payload, type, signature and certificate', async () => {
  const directory = fs.mkdtempSync(path.join(os.tmpdir(), 'nyxid-signing-key-'));
  try {
    execFileSync('openssl', ['req', '-x509', '-newkey', 'ec', '-pkeyopt', 'ec_paramgen_curve:P-256',
      '-nodes', '-keyout', path.join(directory, 'key.pem'), '-out', path.join(directory, 'cert.pem'),
      '-days', '1', '-subj', '/CN=Offline signing compatibility fixture'], {stdio: 'pipe'});
    const privateKey = fs.readFileSync(path.join(directory, 'key.pem'));
    const certificate = fs.readFileSync(path.join(directory, 'cert.pem'), 'utf8');
    const publicKey = new crypto.X509Certificate(certificate).publicKey;
    const data = Buffer.from('{"_type":"https://in-toto.io/Statement/v1","subject":[]}');
    const type = 'application/vnd.in-toto+json'; let captured;
    const encode = object => Buffer.from(JSON.stringify(object)).toString('base64url');
    const token = [encode({alg: 'none'}), encode({iss: 'https://token.actions.githubusercontent.com', sub: 'offline-fixture'}), 'fixture'].join('.');
    const keyHolder = {async sign(bytes) {return {signature: crypto.sign('sha256', bytes, privateKey),
      key: {$case: 'publicKey', publicKey: publicKey.export({type: 'spki', format: 'pem'})}};}};
    const signer = new tools.sign.FulcioSigner({fulcioBaseURL: 'https://fulcio.sigstore.dev',
      identityProvider: {async getToken() {return token;}}, keyHolder});
    let issued = 0;
    signer.ca.createSigningCertificate = async (identity, pem, challenge) => {
      issued++; assert.equal(identity, token);
      assert(crypto.verify('sha256', Buffer.from('offline-fixture'), pem, challenge));
      assert.equal(pem, publicKey.export({type: 'spki', format: 'pem'}));
      return [certificate];
    };
    const witness = new tools.sign.RekorWitness({rekorBaseURL: 'https://rekor.sigstore.dev', entryType: 'dsse', fetchOnConflict: false});
    // Capture the actual producer request before network I/O. Production uses
    // the unmodified public API and live Rekor, authenticated later by W6.
    witness.tlog.createEntry = async proposed => {
      captured = proposed;
      return {logID: 'ab'.repeat(32), logIndex: 0, integratedTime: 1,
        body: Buffer.from(JSON.stringify({kind: 'dsse', apiVersion: '0.0.1'})).toString('base64')};
    };
    const bundler = new tools.sign.DSSEBundleBuilder({certificateChain: false, witnesses: [witness], signer});
    const bundle = tools.bundle.bundleToJSON(await bundler.create({data, type}));
    assert.equal(issued, 1);
    assert.equal(bundle.mediaType, 'application/vnd.dev.sigstore.bundle.v0.3+json');
    const envelope = bundle.dsseEnvelope;
    assert.equal(envelope.signatures.length, 1); assert.equal(envelope.signatures[0].keyid, '');
    assert.equal(captured.apiVersion, '0.0.1'); assert.equal(captured.kind, 'dsse');
    assert.equal(captured.spec.proposedContent.envelope, JSON.stringify({payload: envelope.payload,
      payloadType: type, signatures: [{sig: envelope.signatures[0].sig, keyid: ''}]}));
    assert.deepEqual(captured.spec.proposedContent.verifiers, [Buffer.from(certificate).toString('base64')]);
    const signature = Buffer.from(envelope.signatures[0].sig, 'base64');
    function pae(payloadType, payload) {
      return Buffer.concat([Buffer.from(`DSSEv1 ${Buffer.byteLength(payloadType)} ${payloadType} ${payload.length} `), payload]);
    }
    assert(crypto.verify('sha256', pae(type, data), publicKey, signature));
    assert(!crypto.verify('sha256', pae(type, Buffer.concat([data, Buffer.from('tampered')])), publicKey, signature));
    assert(!crypto.verify('sha256', pae('application/json', data), publicKey, signature));
    // Regression for GHSA-jfc7-64v2-mr8c: UTF-8 type length is bytes, not
    // JavaScript code points, even though our production type is fixed ASCII.
    const core = require(path.join(fs.realpathSync(root), 'node_modules/@sigstore/core'));
    assert.deepEqual(core.dsse.preAuthEncoding('é', data), pae('é', data));
  } finally {fs.rmSync(directory, {recursive: true, force: true});}
});

test('OIDC uses this GitHub job endpoint, sigstore audience, bounded memory and denied redirects', async () => {
  let called = 0;
  const provider = producer.githubIdentity(environment, async (url, options) => {
    called++; assert.equal(url.origin, 'https://fixture.actions.githubusercontent.com');
    assert.equal(url.searchParams.get('audience'), 'sigstore');
    assert.equal(url.searchParams.get('api-version'), '2');
    assert.equal(options.redirect, 'error'); assert.equal(options.method, 'GET');
    assert.equal(options.headers.Authorization, 'Bearer fixture-job-token');
    return new Response(JSON.stringify({value: 'fixture-oidc-result'}));
  });
  assert.equal(await provider.getToken(), 'fixture-oidc-result'); assert.equal(called, 1);
});

test('ambient Sigstore token cannot replace missing GitHub OIDC permission', () => {
  for (const env of [{}, {SIGSTORE_ID_TOKEN: 'fixture-ambient-token'},
    {ACTIONS_ID_TOKEN_REQUEST_URL: environment.ACTIONS_ID_TOKEN_REQUEST_URL},
    {ACTIONS_ID_TOKEN_REQUEST_TOKEN: environment.ACTIONS_ID_TOKEN_REQUEST_TOKEN}]) {
    assert.throws(() => producer.githubIdentity(env), /requires GitHub job OIDC/);
  }
});

test('untrusted OIDC URL cannot receive job credentials', () => {
  for (const url of ['http://fixture.actions.githubusercontent.com/token',
    'https://actions.githubusercontent.com.attacker.invalid/token',
    'https://attacker.invalid/token', 'https://user:pass@fixture.actions.githubusercontent.com/token',
    'https://fixture.actions.githubusercontent.com:8443/token',
    'https://fixture.actions.githubusercontent.com/token#fragment']) {
    assert.throws(() => producer.githubIdentity({...environment, ACTIONS_ID_TOKEN_REQUEST_URL: url}), /untrusted GitHub OIDC endpoint/);
  }
});

test('OIDC HTTP failure, oversized body and malformed token fail closed', async () => {
  for (const response of [new Response('{}', {status: 500}), new Response('x'.repeat(65 * 1024)),
    new Response('invalid'), new Response('{}'), new Response('{"value":false}'),
    new Response(JSON.stringify({value: 'x'.repeat(33 * 1024)}))]) {
    await assert.rejects(producer.githubIdentity(environment, async () => response).getToken());
  }
  await assert.rejects(producer.githubIdentity(environment, async () => {throw new Error('redirect denied');}).getToken());
});

test('production signer construction fixes public endpoints and Rekor format', () => {
  const captured = {}; const identity = {};
  class FulcioSigner {constructor(options) {captured.fulcio = options;}}
  class RekorWitness {constructor(options) {captured.rekor = options;}}
  class DSSEBundleBuilder {constructor(options) {captured.builder = options;}}
  producer.builder({sign: {FulcioSigner, RekorWitness, DSSEBundleBuilder}}, identity);
  assert.equal(captured.fulcio.fulcioBaseURL, 'https://fulcio.sigstore.dev');
  assert.equal(captured.fulcio.identityProvider, identity);
  assert.equal(captured.rekor.rekorBaseURL, 'https://rekor.sigstore.dev');
  assert.equal(captured.rekor.entryType, 'dsse'); assert.equal(captured.rekor.fetchOnConflict, false);
  assert.equal(captured.builder.certificateChain, false); assert.equal(captured.builder.witnesses.length, 1);
  // The real constructors must also accept this reviewed dependency graph.
  assert(producer.builder(tools, identity) instanceof tools.sign.DSSEBundleBuilder);
});

test('rehearsal signer refuses a production manifest before OIDC or network access', async () => {
  const directory = fs.mkdtempSync(path.join(os.tmpdir(), 'nyxid-signing-refusal-'));
  try {
    fs.writeFileSync(path.join(directory, 'official-release-manifest.json'), JSON.stringify({scope: 'images',
      ref: 'refs/heads/main', identity: 'https://github.com/ChronoAIProject/NyxID/.github/workflows/publish-images.yml@refs/heads/main'}));
    await assert.rejects(producer.sign(root, directory), /only the distinct rehearsal/);
    assert(!fs.existsSync(path.join(directory, 'official-release-manifest.sigstore.json')));
  } finally {fs.rmSync(directory, {recursive: true, force: true});}
});
