import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { URL as NodeURL } from "node:url";
import test from "node:test";
import { runInNewContext } from "node:vm";
import ts from "typescript";

import { ApiError } from "./ApiError";

type ApiUrlModule = typeof import("./baseUrl");
type HttpModule = typeof import("./http");
type MobileApiModule = typeof import("./mobileApi");

function loadModule<T>(filename: string, context: Record<string, unknown>): T {
  const source = readFileSync(new NodeURL(filename, import.meta.url), "utf8");
  const js = ts.transpileModule(source, {
    compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 },
  }).outputText;
  const exported = {};
  runInNewContext(js, { exports: exported, ...context });
  return exported as T;
}

function loadResolver(configuredUrl?: string) {
  const env = { EXPO_PUBLIC_API_BASE_URL: configuredUrl };
  const api = loadModule<ApiUrlModule>("./baseUrl.ts", { process: { env } });
  return { api, env };
}

for (const [configuredUrl, expected] of [
  ["https://nyx-api.chrono-ai.fun/api/v1", "https://nyx-api.chrono-ai.fun/api/v1"],
  [" https://api.nyxid.ai/api/v1/// ", "https://api.nyxid.ai/api/v1"],
  ["https://api.nyxid.ai/api/v1/mobile/", "https://api.nyxid.ai/api/v1"],
  ["http://localhost:3001/api/v1", "http://localhost:3001/api/v1"],
  ["https://self-host.example/gateway/api/v1", "https://self-host.example/gateway/api/v1"],
] as const) {
  test(`resolves and joins endpoint paths for ${configuredUrl.trim()}`, () => {
    const { api } = loadResolver(configuredUrl);
    assert.equal(api.getApiBaseUrl(), expected);
    assert.equal(api.NYXID_API_BASE_URL, expected);
    assert.equal(api.buildApiUrl("/users/me"), `${expected}/users/me`);
    assert.equal(api.buildApiUrl("auth/device/preview"), `${expected}/auth/device/preview`);
  });
}

test("keeps one resolved API base for the app lifetime", () => {
  const { api, env } = loadResolver("https://api.nyxid.ai/api/v1/");
  env.EXPO_PUBLIC_API_BASE_URL = "https://different.example/api/v1";
  assert.equal(api.getApiBaseUrl(), "https://api.nyxid.ai/api/v1");
  assert.equal(api.buildApiUrl("/auth/refresh"), "https://api.nyxid.ai/api/v1/auth/refresh");
});

test("missing API configuration fails before a request instead of choosing a default host", () => {
  for (const configuredUrl of [undefined, "", " "]) {
    const { api } = loadResolver(configuredUrl);
    assert.equal(api.NYXID_API_BASE_URL, "");
    assert.throws(() => api.getApiBaseUrl(), /API base URL is not configured/);
    assert.throws(() => api.buildApiUrl("/users/me"), /API base URL is not configured/);
  }
});

test("endpoint inputs cannot select a different host or scheme", () => {
  const { api } = loadResolver("https://api.nyxid.ai/api/v1");
  for (const path of ["https://untrusted.example/api", "http://untrusted.example", "//untrusted.example", "nyxid://login"]) {
    assert.throws(() => api.buildApiUrl(path), /API endpoints must be relative paths/);
  }
});

function loadApiConsumers(api: ApiUrlModule, fetch: typeof globalThis.fetch) {
  const dependencies: Record<string, unknown> = {
    "./ApiError": { ApiError },
    "./baseUrl": api,
    "expo-constants": { default: { expoConfig: { version: "test" } } },
    "../accountDeletionPolicy": { ACCOUNT_DELETION_POLICY_VERSION: "test" },
    "../auth/sessionStore": {
      loadStoredAuthSession: async () => ({ accessToken: "access-before-refresh", refreshToken: "refresh-test" }),
      persistAuthSession: async () => {},
      clearStoredAuthSession: async () => {},
    },
    "./authDeviceApi": {},
    "./idempotency": {},
  };
  const requireDependency = (name: string) => {
    assert.ok(Object.hasOwn(dependencies, name), `Unexpected API dependency: ${name}`);
    return dependencies[name];
  };
  const http = loadModule<HttpModule>("./http.ts", {
    require: requireDependency, fetch, __DEV__: false, setTimeout,
  });
  dependencies["./http"] = http;
  const { mobileApi } = loadModule<MobileApiModule>("./mobileApi.ts", { require: requireDependency });
  return { http, mobileApi };
}

for (const base of ["https://nyx-api.chrono-ai.fun/api/v1", "https://api.nyxid.ai/api/v1"]) {
  test(`HTTP requests, social login, refresh and retries share ${base}`, async () => {
    const { api } = loadResolver(`${base}/mobile/`);
    const requests: { url: string; authorization?: string }[] = [];
    const { http, mobileApi } = loadApiConsumers(api, async (input, init) => {
      requests.push({ url: String(input), authorization: (init?.headers as Record<string, string>)?.Authorization });
      const payload = requests.length === 1
        ? { error: "unauthorized" }
        : requests.length === 2
          ? { access_token: "access-after-refresh", refresh_token: "refresh-next", expires_in: 3600 }
          : { user_id: "test-user" };
      return new Response(JSON.stringify(payload), { status: requests.length === 1 ? 401 : 200 });
    });

    const social = new URL(mobileApi.getSocialAuthorizeUrl("google", "nyxid://auth/social/callback"));
    assert.equal(`${social.origin}${social.pathname}`, `${base}/auth/social/google`);
    assert.equal(social.searchParams.get("redirect_uri"), "nyxid://auth/social/callback");
    const profile = await http.requestJson<{ user_id: string }>("/users/me");
    assert.equal(profile.user_id, "test-user");
    assert.deepEqual(requests.map(({ url }) => url), [
      `${base}/users/me`, `${base}/auth/refresh`, `${base}/users/me`,
    ]);
    assert.equal(requests[2]?.authorization, "Bearer access-after-refresh");
  });
}
