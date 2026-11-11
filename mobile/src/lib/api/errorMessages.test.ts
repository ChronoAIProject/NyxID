import assert from "node:assert/strict";
import test from "node:test";
import { agentKeyLoginError } from "../../features/auth/agentKeyLoginModel";
import { ApiError } from "./ApiError";
import { resolveErrorMessage, resolveAuthDeviceErrorMessage } from "./errorMessages";

test("step-up without an available mobile prompt directs both approval and code mint to the website", () => {
  const error = new ApiError({ statusCode: 403, errorCode: 2003,
    errorKey: "mfa_step_up_required", message: "Server verification instruction" });
  const expected = "Open the NyxID website to verify with your authenticator, then approve the sign-in or create a login code there.";
  assert.equal(resolveErrorMessage(error), expected);
  assert.equal(agentKeyLoginError(error), expected);
  assert.equal(resolveAuthDeviceErrorMessage(error), expected);
});
