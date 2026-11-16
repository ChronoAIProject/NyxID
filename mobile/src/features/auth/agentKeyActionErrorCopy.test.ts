import assert from "node:assert/strict";
import test from "node:test";
import { agentKeyActionErrorCopy } from "./agentKeyLoginModel";
import { LoginRequestError } from "./loginRequestErrors";
import { buildApiError, networkApiError } from "../../lib/api/responseError";

const serverCopy = "NyxID couldn't complete this request. Try again.";
const changedAccess =
  "The key or connection access changed. Refresh and review the updated permissions before approving.";
type Row = [name: string, failure: unknown, step: string, expected: string];
const rows: Row[] =
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
    [
      "5xx with server message",
      buildApiError({ error: "internal_error", message: changedAccess }, 503),
      "confirm",
      serverCopy,
    ],
    ...["options", "review"].map(
      (step): Row => [
        `409 on ${step}`,
        buildApiError(
          { error: "conflict", error_code: 987654, message: changedAccess },
          409,
        ),
        step,
        changedAccess,
      ],
    ),
    [
      "400 on key creation",
      buildApiError(
        {
          error: "validation_error",
          error_code: 987654,
          message: "Choose at least one connection.",
        },
        400,
      ),
      "new",
      "Choose at least one connection.",
    ],
    [
      "403 on review",
      buildApiError(
        {
          error: "forbidden",
          error_code: 987654,
          message: "You no longer have access to this organization.",
        },
        403,
      ),
      "review",
      "You no longer have access to this organization.",
    ],
    [
      "409 on code step",
      buildApiError({ error: "conflict", message: changedAccess }, 409),
      "code",
      serverCopy,
    ],
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
