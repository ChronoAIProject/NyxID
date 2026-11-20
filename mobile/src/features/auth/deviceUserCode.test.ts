import assert from "node:assert/strict";
import test from "node:test";

import {
  extractAuthDeviceUserCodeFromQr,
  normalizeAuthDeviceUserCode,
  formatAuthDeviceUserCode,
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

test("eight-character QR codes retain the installed public format", () => {
  const code = extractAuthDeviceUserCodeFromQr("https://app.nyxid.test/login/device?user_code=ABCD-EFGH", productionTrust);
  assert.equal(code, "ABCDEFGH");
  assert.equal(formatAuthDeviceUserCode(code!), "ABCD-EFGH");
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

test("camera routes only exact staging HTTPS device QR origins", async () => {
  const { extractCameraLoginRequest } = await import("./deviceUserCode");
  const path = "/login/device?user_code=ABCD-EFGH";
  for (const host of ["staging.nyxid.ai", "staging-api.nyxid.ai:443"]) {
    assert.deepEqual(extractCameraLoginRequest(`https://${host}${path}`), {
      kind: "staging-device", userCode: "ABCDEFGH",
    });
  }
  for (const url of [
    `http://staging.nyxid.ai${path}`, `https://staging.nyxid.ai.other.test${path}`,
    `https://user@staging.nyxid.ai${path}`, `https://staging.nyxid.ai:8443${path}`,
    `https://staging.nyxid.ai${path}&user_code=ZYXW-VTSR`,
    "https://staging.nyxid.ai/login/agent-key?user_code=ABCD-EFGH",
  ]) assert.equal(extractCameraLoginRequest(url), null);
});
