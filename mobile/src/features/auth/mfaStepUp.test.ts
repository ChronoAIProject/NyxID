import assert from "node:assert/strict";
import test from "node:test";
import { ApiError } from "../../lib/api/ApiError";
import { resolveAuthDeviceErrorMessage } from "../../lib/api/errorMessages";
import type { AccountProfile } from "../../lib/api/types";
import {
  mobileMfaEnabled,
  isMfaStepUpRequired,
  shouldOfferStepUp,
  completeStepUp,
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
    "Verification required",
  );
  for (const value of [
    error(401, 2003, "mfa_step_up_required"),
    error(403, 2002, "mfa_required"),
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
  assert.equal(await shouldOfferStepUp(error, loadProfile, isCurrent), false);
  assert.equal(profileReads, 0);
});

test("capability fetch discards a changed account and retains the server message when unavailable", async () => {
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
  const pending = shouldOfferStepUp(
    error,
    () =>
      new Promise<AccountProfile>((resolve) => {
        resolveProfile = resolve;
      }),
    () => current,
  );
  current = false;
  resolveProfile(enabled);
  assert.equal(await pending, false);
  assert.equal(
    await shouldOfferStepUp(error, async () => enabled, () => true),
    true,
  );
  assert.equal(
    await shouldOfferStepUp(error, async () => ({} as AccountProfile), () => true),
    false,
  );
  assert.equal(
    await shouldOfferStepUp(
      error,
      async () => {
        throw new Error("profile unavailable");
      },
      () => true,
    ),
    false,
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
