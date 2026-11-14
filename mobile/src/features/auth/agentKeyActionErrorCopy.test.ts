import assert from "node:assert/strict";
import test from "node:test";
import { agentKeyActionErrorCopy } from "./agentKeyLoginModel";
import { LoginRequestError } from "./loginRequestErrors";
import { buildApiError, networkApiError } from "../../lib/api/responseError";

const serverCopy = "NyxID couldn't complete this request. Try again.";
const rows: [name: string, failure: unknown, step: string, expected: string][] =
  [
    ["code step", new Error("local detail"), "code", serverCopy],
    [
      "LoginRequestError",
      new LoginRequestError("malformed"),
      "review",
      "Enter a valid eight-character login code.",
    ],
    [
      "rate_limited",
      { statusCode: 429, errorCode: 11908 },
      "confirm",
      "Too many attempts. Wait before trying again.",
    ],
    [
      "network",
      networkApiError(new TypeError("fetch failed")),
      "confirm",
      "Could not reach the login server. Check your connection and try again.",
    ],
    ["5xx", buildApiError(null, 503), "confirm", serverCopy],
    ["parse failure", buildApiError(null, 200), "confirm", serverCopy],
    [
      "11908",
      { statusCode: 400, errorCode: 11908 },
      "confirm",
      "This key is no longer eligible. Choose another key.",
    ],
    [
      "11909",
      { statusCode: 404, errorCode: 11909 },
      "options",
      "This credential is no longer available.",
    ],
    [
      "local Error",
      new Error("Credential expiry cannot exceed key expiry."),
      "confirm",
      "Credential expiry cannot exceed key expiry.",
    ],
  ];

test("Agent Key action error-copy table preserves resolver and local validation messages", () => {
  for (const [name, failure, step, expected] of rows)
    assert.equal(agentKeyActionErrorCopy(failure, step), expected, name);
});
