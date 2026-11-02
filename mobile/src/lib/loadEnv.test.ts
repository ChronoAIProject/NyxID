import assert from "node:assert/strict";
import { createRequire } from "node:module";
import test from "node:test";
import { extractAuthDeviceUserCodeFromQr } from "../features/auth/deviceUserCode";

type ResolvedProfile = {
  apiBaseUrl: string;
  frontendUrl: string;
  legacyFrontendUrl: string;
  legalBaseUrl: string;
  universalLinkHost: string;
  universalLinkPathPrefix: string;
  iosBundleId: string;
  androidPackage: string;
  iosBuildNumber: string;
  androidVersionCode: string;
};

type ResolveProfile = (
  appEnv: "dev" | "prod",
  env: Record<string, string>,
  options?: { warnOnFrontendUrlFallback?: boolean }
) => ResolvedProfile;

const require = createRequire(import.meta.url);
const { resolveProfile } = require("../../scripts/lib/load-env.js") as {
  resolveProfile: ResolveProfile;
};
const { buildNativeLinks } = require("../../scripts/lib/native-links.js") as {
  buildNativeLinks: (config: Pick<ResolvedProfile, "apiBaseUrl" | "universalLinkHost" | "universalLinkPathPrefix"> & { scheme: string }) => {
    associatedDomains: string[];
    androidIntentFilters: { data: { scheme: string; host?: string; pathPrefix?: string }[] }[];
  };
};

function profileEnv(overrides: Record<string, string> = {}): Record<string, string> {
  return {
    DEV_API_BASE_URL: "http://localhost:3001/api/v1",
    DEV_IOS_BUNDLE_ID: "dev.nyxid.test",
    DEV_ANDROID_PACKAGE: "dev.nyxid.test",
    ...overrides,
  };
}

function captureWarnings(run: () => void): string[] {
  const originalWarn = console.warn;
  const warnings: string[] = [];
  console.warn = (...values: unknown[]) => warnings.push(values.map(String).join(" "));
  try {
    run();
  } finally {
    console.warn = originalWarn;
  }
  return warnings;
}

test("does not warn when the frontend URL is configured explicitly", () => {
  const warnings = captureWarnings(() => {
    const resolved = resolveProfile(
      "dev",
      profileEnv({ DEV_FRONTEND_URL: "http://localhost:3000" }),
      { warnOnFrontendUrlFallback: true }
    );
    assert.equal(resolved.frontendUrl, "http://localhost:3000");
  });

  assert.deepEqual(warnings, []);
});

test("warns when the trusted frontend origin falls back to the legal URL", () => {
  const warnings = captureWarnings(() => {
    const resolved = resolveProfile(
      "dev",
      profileEnv({ DEV_LEGAL_BASE_URL: "http://legal.localhost:3000" }),
      { warnOnFrontendUrlFallback: true }
    );
    assert.equal(resolved.frontendUrl, "http://legal.localhost:3000");
  });

  assert.equal(warnings.length, 1);
  assert.match(warnings[0] ?? "", /DEV_FRONTEND_URL is not explicitly configured/);
  assert.match(warnings[0] ?? "", /moving legal documents cannot break QR scanning/);
});

test("warns loudly when no trusted frontend URL resolves", () => {
  const warnings = captureWarnings(() => {
    const resolved = resolveProfile("dev", profileEnv(), {
      warnOnFrontendUrlFallback: true,
    });
    assert.equal(resolved.frontendUrl, "");
  });

  assert.equal(warnings.length, 1);
  assert.match(warnings[0] ?? "", /NO TRUSTED FRONTEND URL RESOLVED FOR DEV/);
  assert.match(warnings[0] ?? "", /Web QR login scans will be rejected/);
});

test("keeps non-build profile resolution quiet", () => {
  const warnings = captureWarnings(() => {
    const resolved = resolveProfile(
      "dev",
      profileEnv({ DEV_LEGAL_BASE_URL: "http://legal.localhost:3000" })
    );
    assert.equal(resolved.frontendUrl, "http://legal.localhost:3000");
  });

  assert.deepEqual(warnings, []);
});

function productionEnv(overrides: Record<string, string> = {}): Record<string, string> {
  return profileEnv({
    PROD_API_BASE_URL: "https://nyx-api.chrono-ai.fun/api/v1",
    PROD_FRONTEND_URL: "https://nyx.chrono-ai.fun",
    PROD_LEGAL_BASE_URL: "https://nyx.chrono-ai.fun",
    PROD_IOS_BUNDLE_ID: "fun.chrono-ai.nyxid",
    PROD_ANDROID_PACKAGE: "fun.chronoai.nyxid",
    PROD_IOS_BUILD_NUMBER: "37",
    PROD_ANDROID_VERSION_CODE: "37",
    PROD_UNIVERSAL_LINK_HOST: "nyxid.onelink.me",
    PROD_UNIVERSAL_LINK_PATH_PREFIX: "/REzJ",
    PROD_NEW_API_BASE_URL: "https://api.nyxid.ai/api/v1",
    PROD_NEW_FRONTEND_URL: "https://nyxid.ai",
    PROD_NEW_LEGAL_BASE_URL: "https://nyxid.ai",
    ...overrides,
  });
}

for (const flag of ["", "false"]) {
  test(`keeps legacy production domains when the migration flag is ${flag || "unset"}`, () => {
    const resolved = resolveProfile("prod", productionEnv({
      PROD_USE_NEW_DOMAINS: flag,
      PROD_NEW_API_BASE_URL: "invalid-but-unused",
    }));
    assert.equal(resolved.apiBaseUrl, "https://nyx-api.chrono-ai.fun/api/v1");
    assert.equal(resolved.frontendUrl, "https://nyx.chrono-ai.fun");
    assert.equal(resolved.legalBaseUrl, "https://nyx.chrono-ai.fun");
    assert.equal(resolved.legacyFrontendUrl, "");
    assert.equal(resolved.universalLinkHost, "nyxid.onelink.me");
    assert.equal(resolved.universalLinkPathPrefix, "/REzJ");
  });
}

test("switches production URLs while retaining existing native links and app identity", () => {
  const resolved = resolveProfile("prod", productionEnv({
    PROD_USE_NEW_DOMAINS: "true",
    DEV_UNIVERSAL_LINK_PATH_PREFIX: "/dev-only",
  }));
  assert.equal(resolved.apiBaseUrl, "https://api.nyxid.ai/api/v1");
  assert.equal(resolved.frontendUrl, "https://nyxid.ai");
  assert.equal(resolved.legalBaseUrl, "https://nyxid.ai");
  assert.equal(resolved.legacyFrontendUrl, "https://nyx.chrono-ai.fun");
  assert.equal(resolved.universalLinkHost, "nyxid.onelink.me");
  assert.equal(resolved.universalLinkPathPrefix, "/REzJ");
  assert.equal(resolved.iosBundleId, "fun.chrono-ai.nyxid");
  assert.equal(resolved.androidPackage, "fun.chronoai.nyxid");
  assert.equal(resolved.iosBuildNumber, "37");
  assert.equal(resolved.androidVersionCode, "37");
});

test("does not claim the frontend host when no universal-link host is configured", () => {
  const resolved = resolveProfile("prod", productionEnv({
    PROD_USE_NEW_DOMAINS: "true",
    PROD_UNIVERSAL_LINK_HOST: "",
    PROD_UNIVERSAL_LINK_PATH_PREFIX: "",
  }));
  const native = buildNativeLinks({ ...resolved, scheme: "nyxid" });
  assert.deepEqual(native.associatedDomains, ["applinks:api.nyxid.ai"]);
  assert.deepEqual(native.androidIntentFilters, [{
    action: "VIEW",
    data: [{ scheme: "nyxid" }],
    category: ["BROWSABLE", "DEFAULT"],
  }]);
});

for (const useNewDomains of [false, true]) {
  test(`native config preserves OneLink and its path in ${useNewDomains ? "new" : "legacy"} mode`, () => {
    const resolved = resolveProfile("prod", productionEnv({ PROD_USE_NEW_DOMAINS: String(useNewDomains) }));
    const native = buildNativeLinks({ ...resolved, scheme: "nyxid" });
    assert.deepEqual(native.associatedDomains, [
      `applinks:${useNewDomains ? "api.nyxid.ai" : "nyx-api.chrono-ai.fun"}`,
      "applinks:nyxid.onelink.me",
    ]);
    assert.deepEqual(native.androidIntentFilters, [{
      action: "VIEW",
      autoVerify: true,
      data: [{ scheme: "https", host: "nyxid.onelink.me", pathPrefix: "/REzJ" }],
      category: ["BROWSABLE", "DEFAULT"],
    }, {
      action: "VIEW",
      data: [{ scheme: "nyxid" }],
      category: ["BROWSABLE", "DEFAULT"],
    }]);
  });
}

test("accepts new production URLs without requiring legacy API URLs", () => {
  const resolved = resolveProfile("prod", productionEnv({
    PROD_USE_NEW_DOMAINS: "true",
    PROD_API_BASE_URL: "",
    DEV_API_BASE_URL: "",
  }));
  assert.equal(resolved.apiBaseUrl, "https://api.nyxid.ai/api/v1");
});

for (const variable of ["PROD_NEW_API_BASE_URL", "PROD_NEW_FRONTEND_URL", "PROD_NEW_LEGAL_BASE_URL"]) {
  test(`rejects an enabled migration with missing ${variable} instead of falling back`, () => {
    assert.throws(
      () => resolveProfile("prod", productionEnv({ PROD_USE_NEW_DOMAINS: "true", [variable]: " " })),
      new RegExp(`${variable} must be an absolute HTTPS URL`),
    );
  });
}

for (const url of ["not-a-url", "http://api.nyxid.ai/api/v1", "https://user:secret@api.nyxid.ai/api/v1", "https://api.nyxid.ai/api/v1?x=1", "https://api.nyxid.ai/api/v1#fragment"]) {
  test(`rejects invalid new production API URL ${url}`, () => {
    assert.throws(
      () => resolveProfile("prod", productionEnv({ PROD_USE_NEW_DOMAINS: "true", PROD_NEW_API_BASE_URL: url })),
      /PROD_NEW_API_BASE_URL must be/,
    );
  });
}

test("rejects misspelled migration flags", () => {
  assert.throws(
    () => resolveProfile("prod", productionEnv({ PROD_USE_NEW_DOMAINS: "treu" })),
    /PROD_USE_NEW_DOMAINS must be true or false/,
  );
});

test("retains explicit dev URLs while the production migration is enabled", () => {
  const resolved = resolveProfile("dev", productionEnv({
    PROD_USE_NEW_DOMAINS: "true",
    DEV_FRONTEND_URL: "http://localhost:3000",
    DEV_LEGAL_BASE_URL: "http://localhost:3000",
    DEV_UNIVERSAL_LINK_HOST: "dev.example.com",
    DEV_UNIVERSAL_LINK_PATH_PREFIX: "/dev",
  }));
  assert.equal(resolved.apiBaseUrl, "http://localhost:3001/api/v1");
  assert.equal(resolved.frontendUrl, "http://localhost:3000");
  assert.equal(resolved.legalBaseUrl, "http://localhost:3000");
  assert.equal(resolved.legacyFrontendUrl, "");
  assert.equal(resolved.universalLinkHost, "dev.example.com");
  assert.equal(resolved.universalLinkPathPrefix, "/dev");
});

test("uses the selected production domains for the existing dev-profile fallback", () => {
  const warnings = captureWarnings(() => {
    const resolved = resolveProfile("dev", productionEnv({
      PROD_USE_NEW_DOMAINS: "true",
      DEV_API_BASE_URL: "",
    }));
    assert.equal(resolved.apiBaseUrl, "https://api.nyxid.ai/api/v1");
    assert.equal(resolved.frontendUrl, "https://nyxid.ai");
    assert.equal(resolved.legalBaseUrl, "https://nyxid.ai");
  });
  assert.equal(warnings.length, 1);
});

for (const useNewDomains of [false, true]) {
  test(`QR trust follows the selected ${useNewDomains ? "new" : "legacy"} production domains`, () => {
    const resolved = resolveProfile("prod", productionEnv({
      PROD_USE_NEW_DOMAINS: String(useNewDomains),
    }));
    const policy = {
      appScheme: "nyxid",
      webOrigins: [resolved.frontendUrl, resolved.apiBaseUrl, resolved.legacyFrontendUrl, `https://${resolved.universalLinkHost}`],
      allowHttp: false,
    };
    for (const origin of [resolved.frontendUrl, new URL(resolved.apiBaseUrl).origin]) {
      assert.equal(extractAuthDeviceUserCodeFromQr(`${origin}/login/device?user_code=ABCD-EFGH`, policy), "ABCDEFGH");
    }
    const otherFrontend = useNewDomains ? "https://nyx.chrono-ai.fun" : "https://nyxid.ai";
    assert.equal(extractAuthDeviceUserCodeFromQr(`${otherFrontend}/login/device?user_code=ABCD-EFGH`, policy), useNewDomains ? "ABCDEFGH" : null);
    assert.equal(extractAuthDeviceUserCodeFromQr("https://untrusted.example/login/device?user_code=ABCD-EFGH", policy), null);
  });
}
