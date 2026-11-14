import assert from "node:assert/strict";
import test from "node:test";
import {
  evaluateServiceOAuth,
  isServiceOAuthReturn,
  parseServiceOAuthReturn,
  pendingOAuthSchema,
} from "./serviceOAuthModel";

test("authorization needs active server state and changed authorization, including legacy browser dismissal", () => {
  const baseline = { baselineStatus: "active", baselineAuthorizedAt: "before" };
  for (const hint of [undefined, "success"] as const) {
    assert.equal(
      evaluateServiceOAuth(
        baseline,
        { status: "active", last_authorized_at: "before" },
        hint,
      ),
      hint ? "unconfirmed" : "pending",
    );
    assert.equal(
      evaluateServiceOAuth(
        baseline,
        { status: "active", last_authorized_at: "after" },
        hint,
      ),
      "success",
    );
    assert.equal(
      evaluateServiceOAuth(
        { ...baseline, baselineStatus: "pending_auth" },
        { status: "active" },
        hint,
      ),
      "success",
    );
    assert.notEqual(
      evaluateServiceOAuth(baseline, { status: "pending_auth" }, hint),
      "success",
    );
  }
  assert.equal(
    evaluateServiceOAuth(baseline, undefined, "success"),
    "unconfirmed",
  );
});

test("decline, provider failure, server failure and mismatched browser remain distinct", () => {
  const baseline = {
    baselineStatus: "pending_auth",
    baselineAuthorizedAt: null,
  };
  for (const [hint, expected] of [
    ["access_denied", "declined"],
    ["provider_error", "failed"],
    ["exchange_failed", "failed"],
    ["session_mismatch", "session_mismatch"],
  ] as const)
    assert.equal(
      evaluateServiceOAuth(baseline, { status: "failed" }, hint),
      expected,
    );
  assert.equal(evaluateServiceOAuth(baseline, { status: "failed" }), "failed");
});

test("return recognition consumes only the app callback and normalizes untrusted error text", () => {
  assert.equal(
    isServiceOAuthReturn(
      "nyxid://services/oauth-return?status=success",
      "nyxid",
    ),
    true,
  );
  for (const url of [
    "https://evil.example/services/oauth-return",
    "nyxid://services/other",
    "nyxidx://services/oauth-return",
    "NYXID://services/oauth-return?status=success",
  ])
    assert.equal(isServiceOAuthReturn(url, "nyxid"), false);
  assert.equal(
    parseServiceOAuthReturn(
      "nyxid://services/oauth-return?status=error&code=<script>",
      "nyxid",
    ),
    "provider_error",
  );
  assert.equal(
    parseServiceOAuthReturn(
      "nyxid://services/oauth-return?status=error&code=access_denied",
      "nyxid",
    ),
    "access_denied",
  );
  assert.equal(
    pendingOAuthSchema.safeParse({ access_token: "never-persist" }).success,
    false,
  );
});
