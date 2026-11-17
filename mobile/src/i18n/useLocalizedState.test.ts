import assert from 'node:assert/strict';
import test from 'node:test';
import { fileURLToPath } from 'node:url';
import { hookHarness } from '../features/assistant/assistantHookTestHarness';
import { setAppTranslationLanguage, translateApp } from './appTranslation';
import { translate } from './translate';
import { localizedError } from './localizedError';

test('P1-6 active message retains one formatter, preserves interpolated data, replaces and clears it', () => {
  const h = hookHarness<readonly [string | undefined, (v:(()=>string)|undefined)=>void]>({});
  const {useLocalizedState} = h.load(fileURLToPath(new URL('./useLocalizedState.ts',import.meta.url))) as {useLocalizedState:()=>ReturnType<typeof h.value>};
  try {
    h.mount(useLocalizedState);
    const name='Alice / 原样保留 / Save';
    h.value()[1](()=>translateApp('Recent: {{name}}',{name}));h.flush();
    for (const language of ['zh-CN','zh-TW','en'] as const) {
      setAppTranslationLanguage(language);h.render();
      assert.equal(h.value()[0],translate(language,'Recent: {{name}}',{name}));
    }
    h.value()[1](()=>translateApp('Saved'));h.flush();
    setAppTranslationLanguage('zh-TW');h.render();assert.equal(h.value()[0],translate('zh-TW','Saved'));
    h.value()[1](undefined);h.flush();assert.equal(h.value()[0],undefined);
  } finally {h.unmount();setAppTranslationLanguage('en');}
});
test('P1-6 an Error retains its owned source across a later language change', () => {
  const error=localizedError('Recent: {{name}}',{name:'Save'});
  try {for(const language of ['zh-CN','zh-TW','en'] as const){setAppTranslationLanguage(language);assert.equal(error.message,translate(language,'Recent: {{name}}',{name:'Save'}));}}finally{setAppTranslationLanguage('en');}
});
