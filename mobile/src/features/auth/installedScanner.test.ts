import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { runInNewContext } from "node:vm";
import { test } from "node:test";
import { URL as NodeURL } from "node:url";
import ts from "typescript";

// Byte-for-byte pre-#1544 parser (78cb26b3 parent). Only its environment import
// is supplied by this harness; its normalization, URL trust and parsing stay unchanged.
const source = readFileSync(new NodeURL("./__fixtures__/installed-deviceUserCode.ts.txt", import.meta.url), "utf8");
const js = ts.transpileModule(source, { compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 } }).outputText;
const exported: Record<string, unknown> = {};
runInNewContext(js, { exports: exported, require: (name: string) => {
  assert.equal(name, "../../lib/env");
  return { IS_DEV_BUILD: false, NYXID_API_BASE_URL: "https://nyxid.ai/api/v1", NYXID_APP_SCHEME: "nyxid", NYXID_FRONTEND_URL: "https://nyxid.ai", NYXID_UNIVERSAL_LINK_HOST: "nyxid.ai" };
} });
const scanner = exported as {
  extractLoginRequestFromQr(raw: string): { userCode: string } | null;
  normalizeAuthDeviceUserCode(raw: string): string | null;
};

test("installed scanner reads every public generator character in CLI/web QR URLs", () => {
  const backend = readFileSync(new NodeURL("../../../../backend/src/services/auth_device_service.rs", import.meta.url), "utf8");
  const alphabet = backend.match(/AUTH_DEVICE_USER_CODE_ALPHABET:.*?b"([^"]+)"/)![1]!;
  for (let offset = 0; offset < alphabet.length; offset++) {
    for (let position = 0; position < 8; position++) {
      const code = Array.from({ length: 8 }, (_, i) => alphabet[(offset + i + position) % alphabet.length]).join("");
      const formatted = `${code.slice(0, 4)}-${code.slice(4)}`;
      for (const path of ["/login/device", "/login/agent-key"]) {
        const qr = new NodeURL(path, "https://nyxid.ai"); qr.searchParams.set("user_code", formatted);
        assert.equal(scanner.extractLoginRequestFromQr(qr.href)?.userCode, code);
      }
      assert.equal(scanner.normalizeAuthDeviceUserCode(formatted), code);
    }
  }
});
test("installed scanner preserves origin and duplicate guards and rejects nine-character marker", () => {
  for (const payload of ["https://nyxid.ai/login/device?user_code=2-ABCD-EFGH", "https://evil.example/login/device?user_code=ABCD-EFGH", "https://nyxid.ai/login/device?user_code=ABCD-EFGH&user_code=12345678"])
    assert.equal(scanner.extractLoginRequestFromQr(payload), null);
});
