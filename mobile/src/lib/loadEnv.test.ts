import assert from "node:assert/strict";
import { createRequire } from "node:module";
import { mkdtempSync, writeFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { extractAuthDeviceUserCodeFromQr } from "../features/auth/deviceUserCode";
import { loadTestModule } from "./api/apiUrlTestHarness";

type ResolvedProfile = {
  associatedUrls: string[];
  androidHttpsAppLinks: boolean;
  apiBaseUrl: string;
  frontendUrl: string;
  legalBaseUrl: string;
  newApiBaseUrl: string;
  newFrontendUrl: string;
  newLegalBaseUrl: string;
  nativeLinkHosts: "api" | "api+frontend";
  universalLinkHost: string;
  universalLinkPathPrefix: string;
  iosBundleId: string;
  androidPackage: string;
  androidGoogleServicesFile: string;
  iosBuildNumber: string;
  androidVersionCode: string;
};

type ResolveProfile = (
  appEnv: "dev" | "prod",
  env: Record<string, string>,
  options?: {
    warnOnFrontendUrlFallback?: boolean;
    validateAndroidGoogleServices?: boolean;
  },
) => ResolvedProfile;

const require = createRequire(import.meta.url);
const { resolveProfile } = require("../../scripts/lib/load-env.js") as {
  resolveProfile: ResolveProfile;
};
const { buildNativeLinks } = require("../../scripts/lib/native-links.js") as {
  buildNativeLinks: (
    config: Pick<
      ResolvedProfile,
      "universalLinkHost" | "universalLinkPathPrefix"
    > & {
      associatedUrls: string[];
      scheme: string;
      androidHttpsAppLinks?: boolean;
    },
  ) => {
    associatedDomains: string[];
    androidIntentFilters: {
      data: { scheme: string; host?: string; pathPrefix?: string }[];
    }[];
  };
};

function profileEnv(
  overrides: Record<string, string> = {},
): Record<string, string> {
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
  console.warn = (...values: unknown[]) =>
    warnings.push(values.map(String).join(" "));
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
      { warnOnFrontendUrlFallback: true },
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
      { warnOnFrontendUrlFallback: true },
    );
    assert.equal(resolved.frontendUrl, "http://legal.localhost:3000");
  });

  assert.equal(warnings.length, 1);
  assert.match(
    warnings[0] ?? "",
    /DEV_FRONTEND_URL is not explicitly configured/,
  );
  assert.match(
    warnings[0] ?? "",
    /moving legal documents cannot break QR scanning/,
  );
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
      profileEnv({ DEV_LEGAL_BASE_URL: "http://legal.localhost:3000" }),
    );
    assert.equal(resolved.frontendUrl, "http://legal.localhost:3000");
  });

  assert.deepEqual(warnings, []);
});

function productionEnv(
  overrides: Record<string, string> = {},
): Record<string, string> {
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

function nativeLinks(resolved: ResolvedProfile) {
  return buildNativeLinks({
    ...resolved,
    associatedUrls: resolved.associatedUrls,
    scheme: "nyxid",
  });
}

test("embeds both production domain sets while keeping app identity", () => {
  const resolved = resolveProfile(
    "prod",
    productionEnv({ DEV_UNIVERSAL_LINK_PATH_PREFIX: "/dev-only" }),
  );
  assert.equal(resolved.apiBaseUrl, "https://nyx-api.chrono-ai.fun/api/v1");
  assert.equal(resolved.frontendUrl, "https://nyx.chrono-ai.fun");
  assert.equal(resolved.legalBaseUrl, "https://nyx.chrono-ai.fun");
  assert.equal(resolved.newApiBaseUrl, "https://api.nyxid.ai/api/v1");
  assert.equal(resolved.newFrontendUrl, "https://nyxid.ai");
  assert.equal(resolved.newLegalBaseUrl, "https://nyxid.ai");
  assert.equal(resolved.universalLinkHost, "nyxid.onelink.me");
  assert.equal(resolved.universalLinkPathPrefix, "/REzJ");
  assert.equal(resolved.iosBundleId, "fun.chrono-ai.nyxid");
  assert.equal(resolved.androidPackage, "fun.chronoai.nyxid");
  assert.equal(resolved.iosBuildNumber, "37");
  assert.equal(resolved.androidVersionCode, "37");
  assert.equal(resolved.androidGoogleServicesFile, "./google-services.json");
});

test("keeps Firebase configuration profile-local instead of falling back to production", () => {
  const dev = resolveProfile("dev", profileEnv());
  assert.equal(dev.androidGoogleServicesFile, "");

  const prod = resolveProfile("prod", productionEnv());
  assert.equal(prod.androidGoogleServicesFile, "./google-services.json");
});

test("preserves the matching default Firebase file for an existing dev profile", () => {
  const resolved = resolveProfile(
    "dev",
    profileEnv({ DEV_ANDROID_PACKAGE: "fun.chronoai.nyxid" }),
  );
  assert.equal(resolved.androidGoogleServicesFile, "./google-services.json");
});

test("an explicit empty dev Firebase path disables FCM even when the default matches", () => {
  const resolved = resolveProfile(
    "dev",
    profileEnv({
      DEV_ANDROID_PACKAGE: "fun.chronoai.nyxid",
      DEV_GOOGLE_SERVICES_FILE: "",
    }),
  );
  assert.equal(resolved.androidGoogleServicesFile, "");
});

test("an empty production Firebase path preserves the existing FCM default", () => {
  const resolved = resolveProfile(
    "prod",
    productionEnv({ PROD_GOOGLE_SERVICES_FILE: " " }),
    { validateAndroidGoogleServices: true },
  );
  assert.equal(resolved.androidGoogleServicesFile, "./google-services.json");
});

test("rejects a Firebase file that does not contain the active Android package", () => {
  assert.throws(
    () =>
      resolveProfile(
        "dev",
        profileEnv({ DEV_GOOGLE_SERVICES_FILE: "./google-services.json" }),
        { validateAndroidGoogleServices: true },
      ),
    /does not contain a client for dev\.nyxid\.test/,
  );
});

test("rejects a Firebase file with an invalid top-level JSON shape", () => {
  const directory = mkdtempSync(join(tmpdir(), "nyxid-google-services-"));
  const file = join(directory, "google-services.json");
  writeFileSync(file, "null");
  try {
    assert.throws(
      () =>
        resolveProfile("dev", profileEnv({ DEV_GOOGLE_SERVICES_FILE: file }), {
          validateAndroidGoogleServices: true,
        }),
      /does not contain a client for dev\.nyxid\.test/,
    );
  } finally {
    rmSync(directory, { recursive: true, force: true });
  }
});

test("builds without new URLs keep a single domain set", () => {
  const resolved = resolveProfile(
    "prod",
    productionEnv({
      PROD_NEW_API_BASE_URL: "",
      PROD_NEW_FRONTEND_URL: "",
      PROD_NEW_LEGAL_BASE_URL: "",
    }),
  );
  assert.equal(resolved.newApiBaseUrl, "");
  assert.equal(resolved.newFrontendUrl, "");
  assert.equal(resolved.newLegalBaseUrl, "");
  assert.deepEqual(nativeLinks(resolved).associatedDomains, [
    "applinks:nyx-api.chrono-ai.fun",
    "applinks:nyxid.onelink.me",
  ]);
});

test("native config associates both API hosts and preserves OneLink and its path", () => {
  const native = nativeLinks(resolveProfile("prod", productionEnv()));
  assert.deepEqual(native.associatedDomains, [
    "applinks:nyx-api.chrono-ai.fun",
    "applinks:api.nyxid.ai",
    "applinks:nyxid.onelink.me",
  ]);
  assert.deepEqual(native.androidIntentFilters, [
    {
      action: "VIEW",
      autoVerify: true,
      data: [
        { scheme: "https", host: "nyxid.onelink.me", pathPrefix: "/REzJ" },
      ],
      category: ["BROWSABLE", "DEFAULT"],
    },
    {
      action: "VIEW",
      data: [{ scheme: "nyxid" }],
      category: ["BROWSABLE", "DEFAULT"],
    },
  ]);
});

test("native link host defaults preserve today's dev and prod associations", () => {
  for (const [appEnv, env, expectedDomains, expectedFilters] of [
    [
      "dev" as const,
      profileEnv({ DEV_FRONTEND_URL: "http://localhost:3000" }),
      ["applinks:localhost:3001"],
      [
        {
          action: "VIEW",
          data: [{ scheme: "nyxid" }],
          category: ["BROWSABLE", "DEFAULT"],
        },
      ],
    ],
    [
      "prod" as const,
      productionEnv(),
      [
        "applinks:nyx-api.chrono-ai.fun",
        "applinks:api.nyxid.ai",
        "applinks:nyxid.onelink.me",
      ],
      [
        {
          action: "VIEW",
          autoVerify: true,
          data: [
            { scheme: "https", host: "nyxid.onelink.me", pathPrefix: "/REzJ" },
          ],
          category: ["BROWSABLE", "DEFAULT"],
        },
        {
          action: "VIEW",
          data: [{ scheme: "nyxid" }],
          category: ["BROWSABLE", "DEFAULT"],
        },
      ],
    ],
  ] as const) {
    for (const value of [undefined, "", "api"]) {
      const overrides = value === undefined
        ? {}
        : { [`${appEnv.toUpperCase()}_NATIVE_LINK_HOSTS`]: value };
      const resolved = resolveProfile(appEnv, { ...env, ...overrides });
      assert.equal(resolved.nativeLinkHosts, "api");
      const native = nativeLinks(resolved);
      assert.deepEqual(native.associatedDomains, expectedDomains);
      assert.deepEqual(native.androidIntentFilters, expectedFilters);
    }
  }
});

test("api+frontend associates both frontend hosts and shares the path manifest", () => {
  const native = nativeLinks(
    resolveProfile(
      "prod",
      productionEnv({
        PROD_NATIVE_LINK_HOSTS: "api+frontend",
        PROD_ANDROID_HTTPS_APP_LINKS: "true",
      }),
    ),
  );
  assert.deepEqual(native.associatedDomains, [
    "applinks:nyx-api.chrono-ai.fun",
    "applinks:api.nyxid.ai",
    "applinks:nyx.chrono-ai.fun",
    "applinks:nyxid.ai",
    "applinks:nyxid.onelink.me",
  ]);
  const paths = require("../../../shared/native-app-links.json").paths as string[];
  assert.deepEqual(
    native.androidIntentFilters.slice(2).map((filter) =>
      filter.data.map((data) =>
        data.pathPrefix
          ? data.pathPrefix + "*"
          : (data as { path?: string }).path,
      ),
    ),
    [paths, paths, paths, paths],
  );
  assert.deepEqual(
    native.androidIntentFilters.slice(2).map((filter) => filter.data[0]?.host),
    [
      "nyx-api.chrono-ai.fun",
      "api.nyxid.ai",
      "nyx.chrono-ai.fun",
      "nyxid.ai",
    ],
  );
});

function expoConfig(env: Record<string, string>) {
  return loadTestModule<{ default: (context: { config: object }) => {
    ios: { associatedDomains: string[] };
    android: { intentFilters: { data: { host?: string; port?: string; path?: string }[] }[] };
  } }>("../../../app.config.ts", {
    process: { env },
    require: (name: string) => {
      if (name === "./scripts/lib/load-env") return {
        ...require("../../scripts/lib/load-env.js"),
        loadEnv: () => env,
      };
      if (name === "./scripts/lib/native-links") return { buildNativeLinks };
      throw new Error(`Unexpected Expo config dependency: ${name}`);
    },
  }).default({ config: {} });
}
test("Expo consumes the selected host list for iOS entitlements and Android Activity links", () => {
  const config = expoConfig(productionEnv({
    APP_ENV: "prod",
    PROD_NATIVE_LINK_HOSTS: "api+frontend",
    PROD_ANDROID_HTTPS_APP_LINKS: "true",
  }));
  for (const host of ["nyx.chrono-ai.fun", "nyxid.ai"]) {
    assert.ok(config.ios.associatedDomains.includes(`applinks:${host}`));
    assert.ok(config.android.intentFilters.some((filter) =>
      filter.data.some((entry) => entry.host === host && entry.path === "/activity"),
    ));
  }
});

test("api+frontend does not add Android HTTPS filters when opt-in is false", () => {
  const native = nativeLinks(
    resolveProfile(
      "prod",
      productionEnv({ PROD_NATIVE_LINK_HOSTS: "api+frontend" }),
    ),
  );
  assert.deepEqual(native.androidIntentFilters, [
    {
      action: "VIEW",
      autoVerify: true,
      data: [
        { scheme: "https", host: "nyxid.onelink.me", pathPrefix: "/REzJ" },
      ],
      category: ["BROWSABLE", "DEFAULT"],
    },
    {
      action: "VIEW",
      data: [{ scheme: "nyxid" }],
      category: ["BROWSABLE", "DEFAULT"],
    },
  ]);
});

test("api+frontend deduplicates frontend and API hosts", () => {
  const resolved = resolveProfile(
    "prod",
    productionEnv({
      PROD_NATIVE_LINK_HOSTS: "api+frontend",
      PROD_FRONTEND_URL: "https://nyx-api.chrono-ai.fun",
      PROD_NEW_FRONTEND_URL: "https://api.nyxid.ai",
      PROD_ANDROID_HTTPS_APP_LINKS: "true",
    }),
  );
  const native = nativeLinks(resolved);
  assert.deepEqual(native.associatedDomains, [
    "applinks:nyx-api.chrono-ai.fun",
    "applinks:api.nyxid.ai",
    "applinks:nyxid.onelink.me",
  ]);
  assert.deepEqual(
    native.androidIntentFilters.slice(2).map((filter) => filter.data[0]?.host),
    ["nyx-api.chrono-ai.fun", "api.nyxid.ai"],
  );
});

test("http frontend URLs retain iOS association and skip Android HTTPS filters", () => {
  const resolved = resolveProfile(
    "dev",
    profileEnv({
      DEV_NATIVE_LINK_HOSTS: "api+frontend",
      DEV_FRONTEND_URL: "http://localhost:3000",
      DEV_ANDROID_HTTPS_APP_LINKS: "true",
    }),
  );
  const native = nativeLinks(resolved);
  assert.deepEqual(native.associatedDomains, [
    "applinks:localhost:3001",
    "applinks:localhost:3000",
  ]);
  assert.deepEqual(native.androidIntentFilters, [
    {
      action: "VIEW",
      data: [{ scheme: "nyxid" }],
      category: ["BROWSABLE", "DEFAULT"],
    },
  ]);
});

test("api+frontend requires an explicitly configured frontend URL", () => {
  assert.throws(
    () =>
      resolveProfile(
        "dev",
        profileEnv({
          DEV_NATIVE_LINK_HOSTS: "api+frontend",
          DEV_LEGAL_BASE_URL: "http://legal.localhost:3000",
        }),
      ),
    /DEV_FRONTEND_URL.*DEV_NATIVE_LINK_HOSTS.*api\+frontend/,
  );
});

test("api+frontend never claims a frontend URL borrowed from the other build profile", () => {
  for (const appEnv of ["dev", "prod"] as const) {
    const env = productionEnv({
      DEV_NATIVE_LINK_HOSTS: "api+frontend",
      PROD_NATIVE_LINK_HOSTS: "api+frontend",
      DEV_FRONTEND_URL: "https://dev.example.test",
    });
    delete env[`${appEnv.toUpperCase()}_FRONTEND_URL`];
    assert.throws(
      () => resolveProfile(appEnv, env),
      new RegExp(`${appEnv.toUpperCase()}_FRONTEND_URL.*NATIVE_LINK_HOSTS`),
    );
  }
});

for (const value of ["A", "frontend", "api,frontend"]) {
  test(`rejects invalid native link host set ${value}`, () => {
    assert.throws(
      () =>
        resolveProfile(
          "prod",
          productionEnv({ PROD_NATIVE_LINK_HOSTS: value }),
        ),
      /PROD_NATIVE_LINK_HOSTS.*api.*api\+frontend/,
    );
  });
}

test("native link host selection never crosses profiles", () => {
  const prod = resolveProfile(
    "prod",
    productionEnv({ DEV_NATIVE_LINK_HOSTS: "api+frontend" }),
  );
  const dev = resolveProfile(
    "dev",
    productionEnv({ PROD_NATIVE_LINK_HOSTS: "api+frontend" }),
  );
  assert.equal(prod.nativeLinkHosts, "api");
  assert.equal(dev.nativeLinkHosts, "api");
});

test("does not claim a frontend host when no universal-link host is configured", () => {
  const native = nativeLinks(
    resolveProfile(
      "prod",
      productionEnv({
        PROD_UNIVERSAL_LINK_HOST: "",
        PROD_UNIVERSAL_LINK_PATH_PREFIX: "",
      }),
    ),
  );
  assert.deepEqual(native.associatedDomains, [
    "applinks:nyx-api.chrono-ai.fun",
    "applinks:api.nyxid.ai",
  ]);
  assert.deepEqual(native.androidIntentFilters, [
    {
      action: "VIEW",
      data: [{ scheme: "nyxid" }],
      category: ["BROWSABLE", "DEFAULT"],
    },
  ]);
});

for (const variable of [
  "PROD_NEW_API_BASE_URL",
  "PROD_NEW_FRONTEND_URL",
  "PROD_NEW_LEGAL_BASE_URL",
]) {
  test(`rejects a partial new URL set missing ${variable}`, () => {
    assert.throws(
      () => resolveProfile("prod", productionEnv({ [variable]: " " })),
      new RegExp(`${variable} must be an absolute HTTPS URL`),
    );
  });
}

for (const url of [
  "not-a-url",
  "http://api.nyxid.ai/api/v1",
  "https://user:secret@api.nyxid.ai/api/v1",
  "https://api.nyxid.ai/api/v1?x=1",
  "https://api.nyxid.ai/api/v1#fragment",
]) {
  test(`rejects invalid new production API URL ${url}`, () => {
    assert.throws(
      () =>
        resolveProfile("prod", productionEnv({ PROD_NEW_API_BASE_URL: url })),
      /PROD_NEW_API_BASE_URL must be/,
    );
  });
}

test("warns that the retired build flag is no longer read", () => {
  const warnings = captureWarnings(() => {
    const resolved = resolveProfile(
      "prod",
      productionEnv({ PROD_USE_NEW_DOMAINS: "true" }),
    );
    assert.equal(resolved.apiBaseUrl, "https://nyx-api.chrono-ai.fun/api/v1");
  });
  assert.equal(warnings.length, 1);
  assert.match(warnings[0]!, /mobile:new-domains/);
});

test("dev builds never embed the production switch", () => {
  const resolved = resolveProfile(
    "dev",
    productionEnv({
      DEV_FRONTEND_URL: "http://localhost:3000",
      DEV_LEGAL_BASE_URL: "http://localhost:3000",
      DEV_UNIVERSAL_LINK_HOST: "dev.example.com",
      DEV_UNIVERSAL_LINK_PATH_PREFIX: "/dev",
    }),
  );
  assert.equal(resolved.apiBaseUrl, "http://localhost:3001/api/v1");
  assert.equal(resolved.frontendUrl, "http://localhost:3000");
  assert.equal(resolved.newApiBaseUrl, "");
  assert.equal(resolved.universalLinkHost, "dev.example.com");
  assert.equal(resolved.universalLinkPathPrefix, "/dev");
});

test("QR trust accepts both embedded frontends and rejects others", () => {
  const resolved = resolveProfile("prod", productionEnv());
  // Mirrors deviceUserCode.ts defaultWebOrigins: every embedded set plus the link host.
  const policy = {
    appScheme: "nyxid",
    webOrigins: [
      resolved.frontendUrl,
      resolved.newFrontendUrl,
      resolved.apiBaseUrl,
      resolved.newApiBaseUrl,
      `https://${resolved.universalLinkHost}`,
    ],
    allowHttp: false,
  };
  for (const origin of [
    "https://nyx.chrono-ai.fun",
    "https://nyxid.ai",
    "https://api.nyxid.ai",
    "https://nyx-api.chrono-ai.fun",
  ]) {
    assert.equal(
      extractAuthDeviceUserCodeFromQr(
        `${origin}/login/device?user_code=ABCD-EFGH`,
        policy,
      ),
      "ABCDEFGH",
    );
  }
  assert.equal(
    extractAuthDeviceUserCodeFromQr(
      "https://untrusted.example/login/device?user_code=ABCD-EFGH",
      policy,
    ),
    null,
  );
});


test("Android HTTPS App Links opt-in stays profile-local and defaults to today's filters", () => {
  const original = nativeLinks(resolveProfile("prod", productionEnv()));
  const devOnly = nativeLinks(
    resolveProfile("prod", productionEnv({ DEV_ANDROID_HTTPS_APP_LINKS: "true" })),
  );
  assert.deepEqual(devOnly, original);
  assert.equal(original.androidIntentFilters.length, 2);
  const enabled = nativeLinks(
    resolveProfile("prod", productionEnv({ PROD_ANDROID_HTTPS_APP_LINKS: "true" })),
  );
  assert.equal(enabled.androidIntentFilters.length, 4);
  assert.deepEqual(enabled.associatedDomains, original.associatedDomains);
  assert.throws(
    () =>
      resolveProfile(
        "prod",
        productionEnv({ PROD_ANDROID_HTTPS_APP_LINKS: "yes" }),
      ),
    /true or false/,
  );
});

test("one path manifest attaches identically to either host set", () => {
  const paths = require("../../../shared/native-app-links.json").paths as string[];
  for (const associatedUrls of [
    ["https://api.example"],
    ["https://api.example", "https://web.example"],
  ]) {
    const links = buildNativeLinks({
      associatedUrls,
      androidHttpsAppLinks: true,
      universalLinkHost: "",
      universalLinkPathPrefix: "",
      scheme: "nyxid",
    });
    assert.deepEqual(
      links.associatedDomains,
      associatedUrls.map((url) => `applinks:${new URL(url).host}`),
    );
    for (const filter of links.androidIntentFilters.slice(1))
      assert.deepEqual(
        filter.data.map((data) =>
          data.pathPrefix
            ? data.pathPrefix + "*"
            : (data as { path?: string }).path,
        ),
        paths,
      );
  }
});

test("actual Expo config rejects unusable primary and alternate frontend associations only when selected", () => {
  for (const variable of ["PROD_FRONTEND_URL", "PROD_NEW_FRONTEND_URL"]) {
    for (const url of ["not-a-url", "http://app.example", "https://name:secret@app.example", "https://app.example?x=1", "https://app.example#fragment"]) {
      assert.throws(() => expoConfig(productionEnv({
        APP_ENV: "prod",
        PROD_NATIVE_LINK_HOSTS: "api+frontend",
        PROD_ANDROID_HTTPS_APP_LINKS: "true",
        [variable]: url,
      })), new RegExp(variable));
    }
  }
  const apiOnly = expoConfig(productionEnv({ APP_ENV: "prod", PROD_FRONTEND_URL: "not-a-url" }));
  assert.equal(apiOnly.ios.associatedDomains.includes("applinks:not-a-url"), false);
});

test("Expo preserves a valid frontend base path and represents an HTTPS port separately in Android", () => {
  const config = expoConfig(productionEnv({
    APP_ENV: "prod",
    PROD_NATIVE_LINK_HOSTS: "api+frontend",
    PROD_ANDROID_HTTPS_APP_LINKS: "true",
    PROD_FRONTEND_URL: "https://frontend.example:8443/nyxid",
  }));
  assert.ok(config.ios.associatedDomains.includes("applinks:frontend.example:8443"));
  assert.ok(config.android.intentFilters.flatMap((filter) => filter.data).some(
    (data) => data.host === "frontend.example" && data.port === "8443" && data.path === "/activity",
  ));
  assert.equal(config.android.intentFilters.flatMap((filter) => filter.data).some(
    (data) => data.host === "frontend.example:8443",
  ), false);
});
