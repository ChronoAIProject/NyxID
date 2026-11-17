import assert from "node:assert/strict";
import test from "node:test";
import {
  classifyLoginError,
  isLoginDecisionUncertain,
  loginErrorCopy,
  deviceLoginErrorCopy,
} from "./loginRequestErrors";
import { agentKeyActionErrorCopy } from "./agentKeyLoginModel";
import { buildApiError } from "../../lib/api/responseError";
import { resolveAuthDeviceErrorMessage } from "../../lib/api/errorMessages";

const rejected = (status: number, code = 987654) =>
  buildApiError(
    {
      error: "request_rejected",
      error_code: code,
      message: "Untrusted detail",
    },
    status,
  );
const rows = [
  ...[400, 403, 404, 409, 422].map((status) => ({
    name: `${status} unknown code`,
    error: rejected(status),
    kind: "rejected",
    uncertain: false,
  })),
  {
    name: "11908 key ineligible",
    error: rejected(400, 11908),
    kind: "rejected",
    uncertain: false,
  },
  {
    name: "11909 credential missing",
    error: rejected(404, 11909),
    kind: "rejected",
    uncertain: false,
  },
  { name: "500", error: rejected(500), kind: "server", uncertain: true },
  {
    name: "503",
    error: buildApiError(null, 503),
    kind: "server",
    uncertain: true,
  },
  {
    name: "status 0",
    error: buildApiError(null, 0),
    kind: "network",
    uncertain: true,
  },
  {
    name: "AbortError after dispatch",
    error: Object.assign(new Error("aborted"), { name: "AbortError" }),
    kind: "network",
    uncertain: true,
  },
  {
    name: "transport with no status",
    error: new TypeError("fetch failed"),
    kind: "server",
    uncertain: true,
  },
  {
    name: "unparseable response",
    error: buildApiError(null, 200),
    kind: "server",
    uncertain: true,
  },
  {
    name: "unparseable error response",
    error: buildApiError(null, 422),
    kind: "server",
    uncertain: true,
  },
  {
    name: "JSON parse failure",
    error: new SyntaxError("bad JSON"),
    kind: "server",
    uncertain: true,
  },
];
for (const row of rows) {
  test(`decision failure classification and uncertainty: ${row.name}`, () => {
    assert.equal(classifyLoginError(row.error), row.kind);
    assert.equal(isLoginDecisionUncertain(row.error), row.uncertain);
    if (row.kind === "rejected")
      assert.equal(loginErrorCopy(row.error), "NyxID couldn't complete this request. Try again.");
    if (row.kind === "rejected")
      assert.equal(
        deviceLoginErrorCopy(row.error),
        "NyxID couldn't complete this request. Try again.",
      );
  });
}

test("definitive Agent Key rejections retain 11908 and 11909 copy", () => {
  assert.equal(
    agentKeyActionErrorCopy(rejected(400, 11908), "confirm"),
    "This key is no longer eligible. Choose another key.",
  );
  assert.equal(
    agentKeyActionErrorCopy(rejected(404, 11909), "confirm"),
    "This credential is no longer available.",
  );
});

test("Device Login unknown definitive 4xx uses rejection copy and known codes keep their specific message", () => {
  assert.equal(deviceLoginErrorCopy(rejected(422)), "NyxID couldn't complete this request. Try again.");
  const pending = rejected(400, 11202);
  assert.equal(deviceLoginErrorCopy(pending), "This login request is still waiting for approval.");
  assert.equal(deviceLoginErrorCopy(pending), resolveAuthDeviceErrorMessage(pending));
});
