import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { URL as NodeURL } from "node:url";
import { loadTestModule } from "../../lib/api/apiUrlTestHarness";

// React Native runtimes have historically exposed URL/URLSearchParams objects
// whose methods throw. Execute the shipped parser and its real embedded-domain
// configuration without relying on Node's WHATWG URL implementation.
function nativeScanner(useNewDomains: boolean) {
  const process = {
    env: {
      EXPO_PUBLIC_API_BASE_URL: "https://nyx-api.chrono-ai.fun/api/v1",
      EXPO_PUBLIC_FRONTEND_URL: "https://nyx.chrono-ai.fun",
      EXPO_PUBLIC_LEGAL_BASE_URL: "https://nyx.chrono-ai.fun",
      EXPO_PUBLIC_NEW_API_BASE_URL: "https://api.nyxid.ai/api/v1",
      EXPO_PUBLIC_NEW_FRONTEND_URL: "https://nyxid.ai",
      EXPO_PUBLIC_NEW_LEGAL_BASE_URL: "https://nyxid.ai",
      EXPO_PUBLIC_APP_SCHEME: "nyxid",
    },
  };
  const unsupportedUrl = class {
    constructor() { throw new Error("Native URL implementation is unavailable"); }
  };
  const globals = { process, URL: unsupportedUrl, URLSearchParams: unsupportedUrl };
  const domains = loadTestModule<typeof import("../../lib/domains")>(
    "../domains.ts", globals,
  );
  domains.selectNewDomains(useNewDomains);
  const env = loadTestModule<typeof import("../../lib/env")>("../env.ts", {
    ...globals,
    require: (name: string) => {
      assert.equal(name, "./domains");
      return domains;
    },
  });
  return loadTestModule<typeof import("./deviceUserCode")>(
    "../../features/auth/deviceUserCode.ts",
    {
      ...globals,
      require: (name: string) => {
        assert.equal(name, "../../lib/env");
        return env;
      },
    },
  );
}

for (const newDomains of [false, true]) {
  test(`native QR parser accepts both embedded domain sets (new domains ${newDomains})`, () => {
    const scanner = nativeScanner(newDomains);
    for (const origin of [
      "https://nyx.chrono-ai.fun",
      "https://nyx-api.chrono-ai.fun",
      "https://nyxid.ai",
      "https://api.nyxid.ai",
      "nyxid://login",
    ]) {
      const path = origin.startsWith("nyxid:") ? "/device" : "/login/device";
      assert.deepEqual(
        JSON.parse(JSON.stringify(scanner.extractLoginRequestFromQr(
          `${origin}${path}?user_code=ABCD-EFGH`,
        ))),
        { kind: "device", userCode: "ABCDEFGH" },
      );
    }
  });
}

test("native parser accepts every server-generated character and the retained v2 marker", () => {
  const scanner = nativeScanner(true);
  const backend = readFileSync(new NodeURL(
    "../../../../backend/src/services/auth_device_service.rs", import.meta.url,
  ), "utf8");
  const alphabet = backend.match(/AUTH_DEVICE_USER_CODE_ALPHABET:.*?b"([^"]+)"/)![1]!;
  for (const character of alphabet) {
    const code = character.repeat(8);
    for (const path of ["device", "agent-key"]) {
      for (const formatted of [code, `2-${code.slice(0, 4)}-${code.slice(4)}`]) {
        assert.equal(
          scanner.extractLoginRequestFromQr(
            `https://nyxid.ai/login/${path}?user_code=${formatted}`,
          )?.userCode,
          formatted.replaceAll("-", ""),
        );
      }
    }
  }
});

test("native parser keeps origin, protocol, path, code and ambiguity boundaries", () => {
  const scanner = nativeScanner(true);
  for (const payload of [
    "https://evil.example/login/device?user_code=ABCD-EFGH",
    "https://nyxid.ai.evil.example/login/device?user_code=ABCD-EFGH",
    "https://nyxid.ai@evil.example/login/device?user_code=ABCD-EFGH",
    "https://nyxid.ai:444/login/device?user_code=ABCD-EFGH",
    "http://nyxid.ai/login/device?user_code=ABCD-EFGH",
    "https://nyxid.ai/login/other?user_code=ABCD-EFGH",
    "nyxid://evil.example/login/device?user_code=ABCD-EFGH",
    "https://nyxid.ai/login/device?user_code=ABCD-EFGH&user%5Fcode=JKLM-NPQR",
    "https://nyxid.ai/login/device?user_code=%ZZ",
    "https://nyxid.ai/login/device?user_code=ABCD",
    "https://nyxid.ai/login/device?device_code=private-poll-secret",
  ]) {
    assert.equal(scanner.extractLoginRequestFromQr(payload), null, payload);
  }
});
