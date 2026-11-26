import assert from 'node:assert/strict';
import test from 'node:test';
import { wrongScriptCharacters } from './i18nScriptGuard';

test('N9 Taiwan standard 群 and 社群 pass, while genuine Simplified still fails', () => {
  for (const text of ['群', '社群', '社群登入', '社群平台']) {
    assert.deepEqual(wrongScriptCharacters('zh-TW', text), [], text);
  }
  assert.deepEqual(wrongScriptCharacters('zh-TW', '区门'), ['区', '门']);
  assert.deepEqual(wrongScriptCharacters('zh-CN', '區門'), ['區', '門']);
});
