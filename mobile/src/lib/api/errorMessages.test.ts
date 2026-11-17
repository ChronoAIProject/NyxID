import { setAppTranslationLanguage } from "../../i18n/appTranslation";
import assert from "node:assert/strict";
import test from "node:test";
import { agentKeyLoginError } from "../../features/auth/agentKeyLoginModel";
import { ApiError } from "./ApiError";
import { resolveErrorMessage, resolveAuthDeviceErrorMessage } from "./errorMessages";

test("N10 cancellation handling preserves specific messages", () => {
  for (const message of ["auth_session_changed", "cancelled"]) {
    const error = Object.assign(new Error(message), { name: "AbortError" });
    assert.equal(resolveErrorMessage(error), "Request cancelled");
  }
  const upload = Object.assign(new Error("Upload cancelled."), { name: "AbortError" });
  assert.equal(resolveErrorMessage(upload), "Upload cancelled.");
});

test("step-up without an available mobile prompt directs both approval and code mint to the website", () => {
  const error = new ApiError({ statusCode: 403, errorCode: 2003,
    errorKey: "mfa_step_up_required", message: "Server verification instruction" });
  const expected = "Open the NyxID website to verify with your authenticator, then approve the sign-in or create a login code there.";
  assert.equal(resolveErrorMessage(error), expected);
  assert.equal(agentKeyLoginError(error), expected);
  assert.equal(resolveAuthDeviceErrorMessage(error), expected);
});

for (const message of ['Invalid email or password','Invalid MFA code','This account uses social login. Please sign in with your social provider.','Password authentication is unavailable','Authentication changed; sign in again','Unknown server authentication detail']) {
  test(`P2-3 code2000 preserves specific authentication failure: ${message}`, () => {
    const error=new ApiError({statusCode:401,errorCode:2000,errorKey:'authentication_failed',message:`Authentication failed: ${message}`});
    assert.equal(resolveErrorMessage(error),message); assert.equal(error.message,`Authentication failed: ${message}`);
  });
}

test('P2-3 known authentication details translate in both mobile Chinese languages', () => {
  try { for(const language of ['zh-CN','zh-TW'] as const) { setAppTranslationLanguage(language);
    for(const message of ['Invalid email or password','Invalid MFA code','This account uses social login. Please sign in with your social provider.','Password authentication is unavailable','Authentication changed; sign in again']) {
      const error=new ApiError({statusCode:401,errorCode:2000,errorKey:'authentication_failed',message:`Authentication failed: ${message}`});
      const result=resolveErrorMessage(error); assert.notEqual(result,message); assert.match(result,/[\u3400-\u9fff]/);
    }
  }} finally {setAppTranslationLanguage('en');}
});
