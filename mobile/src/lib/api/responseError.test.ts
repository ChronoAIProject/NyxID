import assert from "node:assert/strict";
import test from "node:test";
import { MfaChallengeError } from "../auth/mfaChallenge";
import { ApiError } from "./ApiError";
import {
  buildApiError,
  networkApiError,
  TEMPORARILY_UNAVAILABLE,
  CONNECTION_UNAVAILABLE,
} from "./responseError";
import { executeAuthenticatedRequest } from "./authenticatedTransport";

const headers = new Headers({
  "x-request-id": "request-2275",
  "X-NyxID-Trace-Id": "trace-2275",
});
for (const [status, payload] of [
  [
    503,
    "<html><title>503 Service Temporarily Unavailable</title><center>nginx</center></html>",
  ],
  [502, "upstream connection refused: private address"],
  [504, null],
  [503, '{"error":"unavailable",'],
  [503, { message: "proxy page" }],
  [503, ["proxy page"]],
  [503, { error: "<html>proxy page</html>" }],
] as const) {
  test(`untrusted mobile HTTP ${status} body never becomes UI copy: ${typeof payload}`, () => {
    const error = buildApiError(payload, status, headers);
    assert.ok(error instanceof ApiError);
    assert.equal(error.message, TEMPORARILY_UNAVAILABLE);
    assert.equal(error.statusCode, status);
    assert.equal(error.requestId, "request-2275");
    assert.equal(error.traceId, "trace-2275");
    assert.equal(error.details, undefined);
    assert.doesNotMatch(
      JSON.stringify(error),
      /nginx|private address|proxy page/,
    );
  });
}
test("real NyxID JSON and legacy error-only responses retain their fields", () => {
  const details = { field: "name" };
  const error = buildApiError(
    {
      error: "bad_request",
      error_code: 1000,
      message: "Choose a valid name.",
      details,
    },
    400,
    headers,
  );
  assert.equal(error.message, "Choose a valid name.");
  assert.equal(error.errorKey, "bad_request");
  assert.equal(error.errorCode, 1000);
  assert.equal(error.details, details);
  assert.equal(
    buildApiError({ error: "invalid_mfa_code", message: "Invalid code" }, 401)
      .message,
    "Invalid code",
  );
  assert.equal(buildApiError({ error: "not_found" }, 404).message, "not_found");
});
test("network and cancellation stay distinct and diagnostics cannot become arbitrary prose", () => {
  assert.equal(
    networkApiError(new TypeError("Network request failed: private URL"))
      .message,
    CONNECTION_UNAVAILABLE,
  );
  const abort = Object.assign(new Error("cancelled"), { name: "AbortError" });
  assert.equal(networkApiError(abort), abort);
  const error = buildApiError(null, 503, {
    get: () => "<html>private page</html>",
  });
  assert.equal(error.requestId, undefined);
  assert.equal(error.traceId, undefined);
});
test("native authenticated transport propagates status and correlation without the HTML body", async () => {
  let sends = 0;
  await assert.rejects(
    executeAuthenticatedRequest({
      url: "https://api.test/attachments",
      options: { method: "POST" },
      clientHeaders: {},
      transport: async () => {
        sends++;
        return new Response("<html>nginx</html>", { status: 503, headers });
      },
      loadSession: async () => ({ accessToken: "synthetic", userId: "owner" }),
      refresh: async () => null,
      invalidate: async () => {},
      toError: buildApiError,
    }),
    (error: unknown) => {
      assert.ok(error instanceof ApiError);
      assert.equal(error.message, TEMPORARILY_UNAVAILABLE);
      assert.equal(error.traceId, "trace-2275");
      return true;
    },
  );
  assert.equal(sends, 1, "a mutation is never replayed after a 503");
});

test("keeps OAuth descriptions, numeric legacy errors and step-up diagnostics", () => {
  assert.equal(
    buildApiError(
      { error: "invalid_grant", error_description: "The code expired." },
      400,
    ).message,
    "The code expired.",
  );
  const details = { reason: "mfa_step_up_required" };
  for (const body of [
    {
      error: "mfa_step_up_required",
      error_code: 2003,
      message: "Verify this session",
      details,
    },
    { error_code: 2003, message: "Verify this session", details },
  ]) {
    const error = buildApiError(body, 403);
    assert.equal(error.message, body.message);
    assert.equal(error.errorCode, body.error_code);
    assert.equal(error.details, details);
  }
  const conflict = buildApiError(
    {
      error: "conflict",
      error_code: 1004,
      message: "Conflict: Request expired",
      details: { status: "expired" },
    },
    409,
  );
  assert.equal(conflict.errorCode, 1004);
  assert.deepEqual(conflict.details, { status: "expired" });
});

test("retains MFA verification through the boundary without exposing the challenge", async () => {
  for (const [status, body] of [
    [403, { error_code: 2002, session_token: "synthetic-challenge" }],
    [
      400,
      {
        error: "mfa_required",
        error_code: 2002,
        error_description: "MFA verification required",
        session_token: "synthetic-challenge",
      },
    ],
  ] as const) {
    const error = buildApiError(body, status, headers);
    assert.ok(error instanceof MfaChallengeError);
    assert.equal(error.errorCode, 2002);
    assert.equal(error.traceId, "trace-2275");
    assert.doesNotMatch(JSON.stringify(error), /synthetic-challenge/);
    await error.verify("123456", async (request) => {
      assert.deepEqual(request, {
        code: "123456",
        mfa_token: "synthetic-challenge",
      });
    });
  }
});
