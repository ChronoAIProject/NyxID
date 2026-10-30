import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { pathToFileURL } from 'node:url';
import { desktopFrame } from '../protocol.ts';
import { RED_JPEG, GREEN_PATCH, SESSION, vector } from '../testVectors.ts';

// Run from the repository root with the mobile TSX loader. The reference code
// is temporary; neither captured frames nor an exported bundle touches disk.
const temporary = mkdtempSync(join(tmpdir(), 'nyxid-wire-reference-'));
try {
  const file = join(temporary, 'web-reference.ts');
  writeFileSync(file, execFileSync('git', ['show', 'pr-2064-ref:frontend/src/lib/machine-desktop.ts']));
  const reference = await import(pathToFileURL(file).href);
  const full = vector(new Uint8Array(Buffer.from(RED_JPEG, 'base64')), 9007199254740993n);
  const patch = vector(new Uint8Array(Buffer.from(GREEN_PATCH, 'base64')), 9007199254740994n, {
    width: 8, height: 6, x: 2, y: 2, patchWidth: 2, patchHeight: 2, base: 9007199254740993n,
  });
  for (const buffer of [full, patch]) {
    const native = desktopFrame(buffer, SESSION);
    const web = reference.desktopFrame(buffer, SESSION);
    assert.ok(native); assert.ok(web);
    assert.equal(native.sequence, web.sequence);
    assert.deepEqual(native.rectangle, web.rectangle);
    assert.deepEqual(Buffer.from(native.jpeg), Buffer.from(await web.image.arrayBuffer()));
  }
  console.log('Shared full/NYXD vectors match pr-2064-ref, including u64 above Number.MAX_SAFE_INTEGER.');
} finally {
  rmSync(temporary, { recursive: true, force: true });
}
