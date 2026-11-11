import assert from "node:assert/strict";
import test from "node:test";
import {
  mfaConfirmSchema,
  mfaSetupSchema,
  sessionSchema,
  totpCodeSchema,
  updateProfileSchema,
} from "./accountSchema";

test("profile validation follows backend byte limits and rejects non-http avatars", () => {
  assert.equal(
    updateProfileSchema.safeParse({ display_name: "界".repeat(66) }).success,
    true,
  );
  assert.equal(
    updateProfileSchema.safeParse({ display_name: "界".repeat(67) }).success,
    false,
  );
  assert.deepEqual(updateProfileSchema.parse({ display_name: "  Nyx  " }), {
    display_name: "Nyx",
  });
  assert.deepEqual(updateProfileSchema.parse({ display_name: "" }), {
    display_name: "",
  });
  for (const avatar_url of [
    "",
    "javascript:alert(1)",
    "data:image/png;base64,a",
    "file:///avatar.png",
    "https://",
  ])
    assert.equal(updateProfileSchema.safeParse({ avatar_url }).success, false);
  assert.equal(
    updateProfileSchema.safeParse({
      avatar_url: "https://example.com/avatar.png",
    }).success,
    true,
  );
});

test("current-session identity comes only from the server marker", () => {
  const session = {
    id: "one",
    user_agent: "NyxID mobile",
    created_at: "2026-10-13T00:00:00Z",
    expires_at: "2026-11-13T00:00:00Z",
  };
  assert.equal(sessionSchema.parse(session).is_current, false);
  assert.equal(
    sessionSchema.parse({ ...session, is_current: true }).is_current,
    true,
  );
  assert.equal(
    sessionSchema.safeParse({ ...session, is_current: "true" }).success,
    false,
  );
});

test("TOTP enrollment accepts real otpauth output and login confirmation only accepts six digits", () => {
  assert.equal(
    mfaSetupSchema.safeParse({
      factor_id: "factor",
      secret: "JBSWY3DPEHPK3PXP",
      qr_code_url: "otpauth://totp/NyxID:a@b.com?secret=JBSWY3DPEHPK3PXP",
    }).success,
    true,
  );
  assert.equal(
    mfaSetupSchema.safeParse({
      factor_id: "factor",
      secret: "JBSWY3DPEHPK3PXP",
      qr_code_url: "https://example.com/qr",
    }).success,
    false,
  );
  assert.equal(totpCodeSchema.parse(" 123456 "), "123456");
  for (const code of [
    "",
    "12345",
    "1234567",
    "abc123",
    "recovery-code",
    "１２３４５６",
  ])
    assert.equal(totpCodeSchema.safeParse(code).success, false);
  assert.equal(
    mfaConfirmSchema.safeParse({ message: "Enabled", recovery_codes: [] })
      .success,
    false,
  );
});
