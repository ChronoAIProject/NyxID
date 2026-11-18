import assert from "node:assert/strict";
import { test } from "node:test";
import { ApiError } from "../../lib/api/ApiError";
import { passwordLoginInput, requiresTotp } from "./emailLogin";

test("password whitespace is preserved and the initial login does not send a guessed MFA code", () => {
  assert.deepEqual(
    passwordLoginInput(" person@example.test ", " password ", false, "123456"),
    { email: "person@example.test", password: " password " },
  );
});
test("MFA challenge requires exactly six digits and preserves leading zeroes", () => {
  assert.throws(
    () => passwordLoginInput("person@example.test", "password", true, ""),
    /six-digit/,
  );
  assert.throws(
    () =>
      passwordLoginInput(
        "person@example.test",
        "password",
        true,
        "recovery-code",
      ),
    /six-digit/,
  );
  assert.equal(
    passwordLoginInput("person@example.test", "password", true, " 012345 ")
      .mfaCode,
    "012345",
  );
});
test("only the actual MFA-required error enters the authenticator path", () => {
  assert.equal(
    requiresTotp(
      new ApiError({
        errorKey: "mfa_required",
        errorCode: 2002,
        statusCode: 403,
        message: "MFA verification required",
      }),
    ),
    true,
  );
  assert.equal(
    requiresTotp(
      new ApiError({
        errorKey: "forbidden",
        errorCode: 2003,
        statusCode: 403,
        message: "Access forbidden",
      }),
    ),
    false,
  );
});
