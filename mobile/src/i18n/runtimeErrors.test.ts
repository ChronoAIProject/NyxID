import assert from 'node:assert/strict';
import test from 'node:test';
import { setAppTranslationLanguage } from './appTranslation';
import { ApiError } from '../lib/api/ApiError';
import { resolveAuthDeviceErrorMessage, resolveErrorMessage } from '../lib/api/errorMessages';

test('P1-6 mapped API errors and connection fallbacks follow the selected app language', () => {
  try {
    for (const language of ['zh-CN', 'zh-TW'] as const) {
      setAppTranslationLanguage(language);
      for (const code of [1001,1005,2001,2003,7000,7001,8001,8002,11200,11201,11202,11203,11204,11205,11206,11207]) {
        const error = new ApiError({statusCode:400,errorCode:code,errorKey:'fixture',message:'Raw server fixture'});
        assert.match(resolveErrorMessage(error), /[\u3400-\u9fff]/, `${language} ${code}`);
      }
      assert.match(resolveAuthDeviceErrorMessage(new Error('network')), /[\u3400-\u9fff]/);
      assert.match(resolveErrorMessage(new Error('Invalid MFA code')), /[\u3400-\u9fff]/);
      assert.match(resolveErrorMessage(null), /[\u3400-\u9fff]/);
    }
  } finally { setAppTranslationLanguage('en'); }
});
