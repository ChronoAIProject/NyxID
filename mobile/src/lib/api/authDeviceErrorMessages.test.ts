import assert from "node:assert/strict";
import test from "node:test";

import { ApiError } from "./ApiError";
import { resolveAuthDeviceErrorMessage } from "./errorMessages";

const CONNECTION_ERROR =
  "Couldn't reach NyxID. Check your connection and try again.";

test("auth device errors do not expose transport details", () => {
  assert.equal(
    resolveAuthDeviceErrorMessage(new Error("request_failed_502")),
    CONNECTION_ERROR,
  );
});

test("auth device errors do not expose unmapped API messages", () => {
  assert.equal(
    resolveAuthDeviceErrorMessage(
      new ApiError({
        errorKey: "upstream_failure",
        errorCode: 19999,
        statusCode: 502,
        message: "Proxy returned an invalid upstream response",
      }),
    ),
    CONNECTION_ERROR,
  );
});

test("auth device errors preserve mapped device-code messages", () => {
  assert.equal(
    resolveAuthDeviceErrorMessage(
      new ApiError({
        errorKey: "auth_device_code_expired",
        errorCode: 11201,
        statusCode: 400,
        message: "Raw backend message",
      }),
    ),
    "This login request has expired.",
  );
});
