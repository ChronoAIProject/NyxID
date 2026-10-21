import assert from 'node:assert/strict';
import test from 'node:test';
import { frontendSourceIdentity } from './source-identity.ts';

const commit = 'a'.repeat(40);
test('exported frontend retains exact full commit and epoch', () => {
  assert.deepEqual(frontendSourceIdentity({ SOURCE_COMMIT: commit, SOURCE_DATE_EPOCH: '1792365742' }), {
    schema: 'nyxid.frontend-build-identity.v1', commit, source_date_epoch: 1792365742, source_state: 'exported',
  });
});
test('ordinary local and CI builds remain unverified', () => {
  assert.equal(frontendSourceIdentity({}).source_state, 'unverified');
  assert.equal(frontendSourceIdentity({ GITHUB_SHA: commit }).source_state, 'unverified');
});
test('missing, short, uppercase and conflicting commits fail', () => {
  for (const value of ['', 'unknown', 'a'.repeat(12), 'A'.repeat(40), commit + '\n']) {
    assert.throws(() => frontendSourceIdentity({ SOURCE_COMMIT: value, SOURCE_DATE_EPOCH: '1' }));
  }
  assert.throws(() => frontendSourceIdentity({ SOURCE_COMMIT: commit, GITHUB_SHA: 'b'.repeat(40), SOURCE_DATE_EPOCH: '1' }));
  assert.throws(() => frontendSourceIdentity({ SOURCE_COMMIT: commit, NYXID_GIT_HASH: 'b'.repeat(40), SOURCE_DATE_EPOCH: '1' }));
});
test('missing, noncanonical and unsupported epochs fail', () => {
  for (const value of [undefined, '', '0', '-1', '01', '1.5', '253402300800', '1\n']) {
    assert.throws(() => frontendSourceIdentity({ SOURCE_COMMIT: commit, SOURCE_DATE_EPOCH: value }));
  }
  assert.throws(() => frontendSourceIdentity({ SOURCE_DATE_EPOCH: '1' }));
});
