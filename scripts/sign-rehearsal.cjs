#!/usr/bin/env node
'use strict';

// Public Sigstore signing APIs, with the same DSSE/Rekor serialization as the
// pinned GitHub producer. This tool has no GitHub attestation-upload API.
const fs = require('node:fs');
const path = require('node:path');

function libraries(root) {
  const base = fs.realpathSync(root);
  for (const [name, version] of Object.entries({
    '@sigstore/sign': '3.0.0', '@sigstore/bundle': '3.0.0',
    '@sigstore/core': '3.2.1', '@sigstore/protobuf-specs': '0.3.2',
  })) {
    const installed = JSON.parse(fs.readFileSync(path.join(base, 'node_modules', name, 'package.json')));
    if (installed.name !== name || installed.version !== version) throw new Error('reviewed signing library version mismatch');
  }
  return {
    sign: require(path.join(base, 'node_modules/@sigstore/sign')),
    bundle: require(path.join(base, 'node_modules/@sigstore/bundle')),
  };
}

function githubIdentity(environment = process.env, request = fetch) {
  // Do not use the library's ambient environment-token fallback. Only this
  // job's GitHub OIDC endpoint can supply the signing identity.
  if (!environment.ACTIONS_ID_TOKEN_REQUEST_URL || !environment.ACTIONS_ID_TOKEN_REQUEST_TOKEN) {
    throw new Error('rehearsal requires GitHub job OIDC permissions');
  }
  const url = new URL(environment.ACTIONS_ID_TOKEN_REQUEST_URL);
  if (url.protocol !== 'https:' || !url.hostname.endsWith('.actions.githubusercontent.com') ||
      url.username || url.password || (url.port && url.port !== '443') || url.hash) {
    throw new Error('untrusted GitHub OIDC endpoint');
  }
  url.searchParams.set('audience', 'sigstore');
  return {async getToken() {
    const response = await request(url, {
      method: 'GET', redirect: 'error', signal: AbortSignal.timeout(30_000),
      headers: {Accept: 'application/json', Authorization: `Bearer ${environment.ACTIONS_ID_TOKEN_REQUEST_TOKEN}`},
    });
    if (!response.ok) throw new Error('GitHub OIDC issuance failed');
    const chunks = []; let size = 0;
    for await (const chunk of response.body) {
      size += chunk.length;
      if (size > 64 * 1024) throw new Error('oversized GitHub OIDC response');
      chunks.push(Buffer.from(chunk));
    }
    const value = JSON.parse(Buffer.concat(chunks)).value;
    if (typeof value !== 'string' || !value || value.length > 32 * 1024) throw new Error('invalid GitHub OIDC response');
    return value;
  }};
}

function builder(tools, identityProvider) {
  const signer = new tools.sign.FulcioSigner({
    fulcioBaseURL: 'https://fulcio.sigstore.dev', identityProvider,
    timeout: 30_000, retry: {retries: 2},
  });
  const witness = new tools.sign.RekorWitness({
    rekorBaseURL: 'https://rekor.sigstore.dev', entryType: 'dsse',
    fetchOnConflict: false, timeout: 30_000, retry: {retries: 2},
  });
  return new tools.sign.DSSEBundleBuilder({signer, witnesses: [witness], certificateChain: false});
}

async function sign(root, proof) {
  const tools = libraries(root);
  const document = JSON.parse(fs.readFileSync(path.join(proof, 'official-release-manifest.json')));
  if (document.scope !== 'rehearsal' || document.identity !==
      'https://github.com/ChronoAIProject/NyxID/.github/workflows/security-release-rehearsal.yml@' + document.ref) {
    throw new Error('only the distinct rehearsal manifest can be signed here');
  }
  const source = path.join(proof, 'statement.json');
  const metadata = fs.lstatSync(source);
  if (!metadata.isFile() || metadata.size > 256 * 1024) throw new Error('bounded regular rehearsal statement required');
  const data = fs.readFileSync(source);
  const bundler = builder(tools, githubIdentity());
  const bundle = tools.bundle.bundleToJSON(await bundler.create({data, type: 'application/vnd.in-toto+json'}));
  // Cryptographic acceptance is exclusively the later mandatory exact-W6
  // invocation. A successful signing API response is insufficient.
  fs.writeFileSync(path.join(proof, 'official-release-manifest.sigstore.json'), JSON.stringify(bundle) + '\n', {flag: 'wx', mode: 0o600});
}

module.exports = {libraries, githubIdentity, builder, sign};
if (require.main === module) {
  if (process.argv.length !== 4) throw new Error('usage: sign-rehearsal.cjs LOCKED-TOOLS-ROOT PROOF-DIRECTORY');
  sign(path.resolve(process.argv[2]), path.resolve(process.argv[3])).catch(() => {
    // An upstream exception may include HTTP internals. Keep OIDC credentials
    // out of diagnostics and let the later stages remain blocked.
    console.error('Rehearsal signing failed; no publication is authorized');
    process.exitCode = 1;
  });
}
