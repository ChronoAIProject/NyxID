import assert from "node:assert/strict";
import test from "node:test";

import {
  extractAuthDeviceUserCodeFromQr,
  normalizeAuthDeviceUserCode,
  formatAuthDeviceUserCode,
  supportsRestrictedDeviceLogin,
  type AuthDeviceQrTrustPolicy,
} from "./deviceUserCode";

const productionTrust: AuthDeviceQrTrustPolicy = {
  appScheme: "nyxid",
  webOrigins: ["https://app.nyxid.test", "https://nyxid.onelink.test"],
  allowHttp: false,
};

const developmentTrust: AuthDeviceQrTrustPolicy = {
  ...productionTrust,
  webOrigins: [...productionTrust.webOrigins, "http://localhost:3000"],
  allowHttp: true,
};

test("v2 QR and manual codes preserve protocol and restricted grant eligibility", () => {
  const code = extractAuthDeviceUserCodeFromQr("https://app.nyxid.test/login/device?user_code=2-ABCD-EFGH", productionTrust);
  assert.equal(code, "2ABCDEFGH");
  assert.equal(formatAuthDeviceUserCode(code!), "2-ABCD-EFGH");
  assert.equal(supportsRestrictedDeviceLogin(code!), true);
  assert.equal(supportsRestrictedDeviceLogin("ABCD-EFGH"), false);
  assert.equal(supportsRestrictedDeviceLogin("3-ABCD-EFGH"), false);
});

test("typing and pasting either login code format preserves the complete request", () => {
  for (const [compact, formatted, restricted] of [
    ["WJEGRKAM", "WJEG-RKAM", false],
    ["2ABCDEFG", "2ABC-DEFG", false],
    ["2WJEGRKAM", "2-WJEG-RKAM", true],
    ["22ABCDEFG", "2-2ABC-DEFG", true],
  ] as const) {
    let typed = "";
    for (const character of compact) {
      typed = formatAuthDeviceUserCode(typed + character);
    }
    assert.equal(typed, formatted);
    assert.equal(normalizeAuthDeviceUserCode(typed), compact);
    assert.equal(supportsRestrictedDeviceLogin(typed), restricted);
    assert.equal(formatAuthDeviceUserCode(formatted.toLowerCase()), formatted);

    const deleted = formatAuthDeviceUserCode(typed.slice(0, -1));
    assert.equal(formatAuthDeviceUserCode(deleted + compact.at(-1)), formatted);
  }
});

test("formatting an invalid paste never truncates it into an accepted login code", () => {
  for (const raw of [
    "WJEG-RKAMZ",
    "2-WJEG-RKAMZ",
    "2-WJEG-RKAM!",
    "3-WJEG-RKAM",
    "WJEG-RKAM\n",
  ]) {
    assert.equal(normalizeAuthDeviceUserCode(raw), null);
    assert.equal(normalizeAuthDeviceUserCode(formatAuthDeviceUserCode(raw)), null, raw);
  }
});

test("QR and custom-scheme links preserve v2 codes and reject overlong codes", () => {
  for (const prefix of [
    "https://app.nyxid.test/login/device",
    "nyxid://login/device",
  ]) {
    assert.equal(
      extractAuthDeviceUserCodeFromQr(`${prefix}?user_code=2-WJEG-RKAM`, productionTrust),
      "2WJEGRKAM",
    );
    assert.equal(
      extractAuthDeviceUserCodeFromQr(`${prefix}?user_code=2-WJEG-RKAMZ`, productionTrust),
      null,
    );
  }
});

test("extracts a code from the trusted HTTPS device-login URL", () => {
  assert.equal(
    extractAuthDeviceUserCodeFromQr(
      "https://app.nyxid.test/login/device?user_code=ABCD-EFGH",
      productionTrust
    ),
    "ABCDEFGH"
  );
});

test("accepts HTTP only for an explicitly trusted development origin", () => {
  const url = "http://localhost:3000/login/device?user_code=ABCD-EFGH";
  assert.equal(extractAuthDeviceUserCodeFromQr(url, developmentTrust), "ABCDEFGH");
  assert.equal(extractAuthDeviceUserCodeFromQr(url, productionTrust), null);
});

test("extracts codes from both NyxID custom-scheme deep-link shapes", () => {
  for (const url of [
    "nyxid://login/device?user_code=ABCD-EFGH",
    "nyxid:///login/device?user_code=ABCD-EFGH",
  ]) {
    assert.equal(extractAuthDeviceUserCodeFromQr(url, productionTrust), "ABCDEFGH");
  }
});

test("rejects a web URL from an untrusted or authority-confused host", () => {
  assert.equal(
    extractAuthDeviceUserCodeFromQr(
      "https://evil.test/login/device?user_code=ABCD-EFGH",
      productionTrust
    ),
    null
  );
  assert.equal(
    extractAuthDeviceUserCodeFromQr(
      "https://app.nyxid.test@evil.test/login/device?user_code=ABCD-EFGH",
      productionTrust
    ),
    null
  );
});

test("rejects the wrong login path", () => {
  assert.equal(
    extractAuthDeviceUserCodeFromQr(
      "https://app.nyxid.test/login/other?user_code=ABCD-EFGH",
      productionTrust
    ),
    null
  );
});

test("rejects a URL without user_code", () => {
  assert.equal(
    extractAuthDeviceUserCodeFromQr(
      "https://app.nyxid.test/login/device?code=ABCD-EFGH",
      productionTrust
    ),
    null
  );
});

test("rejects duplicate user_code parameters, including encoded keys", () => {
  assert.equal(
    extractAuthDeviceUserCodeFromQr(
      "https://app.nyxid.test/login/device?user_code=ABCD-EFGH&user%5Fcode=JKLM-NPQR",
      productionTrust
    ),
    null
  );
});

test("rejects malformed and non-URL input", () => {
  assert.equal(extractAuthDeviceUserCodeFromQr("not a URL", productionTrust), null);
  assert.equal(
    extractAuthDeviceUserCodeFromQr(
      "https://app.nyxid.test/login/device?user_code=%ZZ",
      productionTrust
    ),
    null
  );
});

test("normalizes ambiguous characters I/L to 1, O to 0, and U to V", () => {
  assert.equal(normalizeAuthDeviceUserCode("iLou-aubc"), "110VAVBC");
});
