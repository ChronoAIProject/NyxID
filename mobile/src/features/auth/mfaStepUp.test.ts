import assert from "node:assert/strict";
import test from "node:test";
import { ApiError } from "../../lib/api/ApiError";
import { resolveAuthDeviceErrorMessage } from "../../lib/api/errorMessages";
import type { AccountProfile } from "../../lib/api/types";
import {
  mobileMfaEnabled,
  isMfaStepUpRequired,
  decideStepUp,
  completeStepUp,
  stepUpScopeIsCurrent,
} from "./mfaStepUp";

test("MFA UI defaults off for old backends and follows per-person capabilities", () => {
  for (const capabilities of [
    undefined,
    {},
    { enabled_features: [] },
    { enabled_features: ["other:flag"] },
  ]) {
    assert.equal(mobileMfaEnabled({ capabilities } as AccountProfile), false);
  }
  assert.equal(
    mobileMfaEnabled({
      capabilities: { enabled_features: ["auth:mobile-mfa"] },
    } as AccountProfile),
    true,
  );
});
test("step-up recognizes the typed 403 without mistaking session expiry or login challenges", () => {
  const error = (statusCode: number, errorCode: number, errorKey: string) =>
    new ApiError({
      statusCode,
      errorCode,
      errorKey,
      message: "Verification required",
    });
  assert.equal(
    isMfaStepUpRequired(error(403, 2003, "mfa_step_up_required")),
    true,
  );
  assert.equal(
    resolveAuthDeviceErrorMessage(error(403, 2003, "mfa_step_up_required")),
    "Open the NyxID website to verify with your authenticator, then approve the sign-in or create a login code there.",
  );
  for (const value of [
    error(401, 2003, "mfa_step_up_required"),
    error(403, 2002, "mfa_required"),
    error(403, 2003, "other_error"),
    null,
    new Error("step-up"),
  ])
    assert.equal(isMfaStepUpRequired(value), false);
});

test("a late step-up refusal cannot prompt under a replacement login", async () => {
  const error = new ApiError({
    statusCode: 403,
    errorCode: 2003,
    errorKey: "mfa_step_up_required",
    message: "Verification required",
  });
  let generation = 0;
  const originatingGeneration = generation;
  const isCurrent = () => generation === originatingGeneration;
  let profileReads = 0;
  const loadProfile = async () => {
    profileReads += 1;
    return {
      capabilities: { enabled_features: ["auth:mobile-mfa"] },
    } as AccountProfile;
  };
  generation += 1;
  assert.equal(await decideStepUp(error, loadProfile, isCurrent), "discard");
  assert.equal(profileReads, 0);
});

test("capability fetch discards a changed account and selects website guidance when unavailable", async () => {
  const error = new ApiError({
    statusCode: 403,
    errorCode: 2003,
    errorKey: "mfa_step_up_required",
    message: "Verification required",
  });
  const enabled = {
    capabilities: { enabled_features: ["auth:mobile-mfa"] },
  } as AccountProfile;
  let current = true;
  let resolveProfile!: (profile: AccountProfile) => void;
  const pending = decideStepUp(
    error,
    () =>
      new Promise<AccountProfile>((resolve) => {
        resolveProfile = resolve;
      }),
    () => current,
  );
  current = false;
  resolveProfile(enabled);
  assert.equal(await pending, "discard");
  assert.equal(
    await decideStepUp(error, async () => enabled, () => true),
    "prompt",
  );
  assert.equal(
    await decideStepUp(error, async () => ({} as AccountProfile), () => true),
    "message",
  );
  assert.equal(
    await decideStepUp(
      error,
      async () => {
        throw new Error("profile unavailable");
      },
      () => true,
    ),
    "message",
  );
});
for (const surface of [
  "device approval",
  "app identity approval",
  "Agent Key approval",
  "login-code mint",
]) {
  test(`${surface} retries only after verification and discards stale requests`, async () => {
    const calls: string[] = [];
    const verify = async (code: string) => {
      assert.equal(code, "input");
      calls.push("verify");
    };
    const retry = async () => {
      calls.push("retry");
    };
    await completeStepUp("input", verify, retry, () => true);
    assert.deepEqual(calls, ["verify", "retry"]);
    calls.length = 0;
    await assert.rejects(
      completeStepUp(
        "input",
        async () => {
          throw new Error("wrong code");
        },
        retry,
        () => true,
      ),
    );
    assert.deepEqual(calls, []);
    await completeStepUp("input", verify, retry, () => false);
    assert.deepEqual(calls, []);
    let current = true;
    await completeStepUp(
      "input",
      async () => {
        current = false;
      },
      retry,
      () => current,
    );
    assert.deepEqual(calls, []);
  });
}


test("step-up decisions cover unrelated errors and stale scope/generation on every async exit", async () => {
  const origin = { scope: "approval-1", generation: 1 };
  assert.equal(stepUpScopeIsCurrent(origin, "approval-1", 1), true);
  assert.equal(stepUpScopeIsCurrent(origin, "approval-2", 1), false);
  assert.equal(stepUpScopeIsCurrent(origin, "approval-1", 2), false);
  assert.equal(stepUpScopeIsCurrent(origin, null, 1), false);
  assert.equal(await decideStepUp(new Error("unrelated"), async () => {
    assert.fail("must not fetch capabilities");
  }, () => true), "unhandled");
  let current = true;
  assert.equal(await decideStepUp(new ApiError({statusCode: 403, errorCode: 2003,
    errorKey: "mfa_step_up_required", message: "step up"}), async () => {
      current = false;
      throw new Error("account changed while fetching");
    }, () => current), "discard");
});
