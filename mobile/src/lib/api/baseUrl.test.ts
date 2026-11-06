import assert from "node:assert/strict";
import test from "node:test";
import { loadApiConsumers, loadTestModule } from "./apiUrlTestHarness";

type ApiUrlModule = typeof import("./baseUrl");
type DomainsModule = typeof import("../domains");

function loadResolver(configuredUrl?: string, extraEnv: Record<string, string> = {}) {
  const env: Record<string, string | undefined> = { EXPO_PUBLIC_API_BASE_URL: configuredUrl, ...extraEnv };
  const domains = loadTestModule<DomainsModule>("../domains.ts", { process: { env } });
  const api = loadTestModule<ApiUrlModule>("./baseUrl.ts", {
    require: (name: string) => {
      assert.equal(name, "../domains");
      return domains;
    },
  });
  return { api, domains, env };
}

const NEW_DOMAIN_ENV = {
  EXPO_PUBLIC_FRONTEND_URL: "https://nyx.chrono-ai.fun",
  EXPO_PUBLIC_LEGAL_BASE_URL: "https://nyx.chrono-ai.fun",
  EXPO_PUBLIC_NEW_API_BASE_URL: "https://api.nyxid.ai/api/v1/",
  EXPO_PUBLIC_NEW_FRONTEND_URL: "https://nyxid.ai",
  EXPO_PUBLIC_NEW_LEGAL_BASE_URL: "https://nyxid.ai",
};

for (const [configuredUrl, expected] of [
  ["https://nyx-api.chrono-ai.fun/api/v1", "https://nyx-api.chrono-ai.fun/api/v1"],
  [" https://api.nyxid.ai/api/v1/// ", "https://api.nyxid.ai/api/v1"],
  ["https://api.nyxid.ai/api/v1/mobile/", "https://api.nyxid.ai/api/v1"],
  ["http://localhost:3001/api/v1", "http://localhost:3001/api/v1"],
  ["https://self-host.example/gateway/api/v1", "https://self-host.example/gateway/api/v1"],
] as const) {
  test(`resolves and joins endpoint paths for ${configuredUrl.trim()}`, () => {
    const { api, domains } = loadResolver(configuredUrl);
    assert.equal(api.getApiBaseUrl(), expected);
    assert.equal(domains.activeDomains().apiBaseUrl, expected);
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
    const { api, domains } = loadResolver(configuredUrl);
    assert.equal(domains.activeDomains().apiBaseUrl, "");
    assert.throws(() => api.getApiBaseUrl(), /API base URL is not configured/);
    assert.throws(() => api.buildApiUrl("/users/me"), /API base URL is not configured/);
  }
});

test("the launch selection switches every request to the embedded new domain set", () => {
  const { api, domains } = loadResolver("https://nyx-api.chrono-ai.fun/api/v1", NEW_DOMAIN_ENV);
  assert.throws(() => api.buildApiUrl("/users/me"), /Domains have not been selected/);
  domains.selectNewDomains(true);
  assert.equal(api.buildApiUrl("/users/me"), "https://api.nyxid.ai/api/v1/users/me");
  assert.equal(domains.getFrontendUrl(), "https://nyxid.ai");
  assert.equal(domains.getLegalBaseUrl(), "https://nyxid.ai");
  assert.equal(domains.activeDomains(), domains.DOMAIN_SETS.new);
  domains.selectNewDomains(false);
  assert.equal(api.buildApiUrl("/users/me"), "https://nyx-api.chrono-ai.fun/api/v1/users/me");
  assert.equal(domains.getFrontendUrl(), "https://nyx.chrono-ai.fun");
});

test("consumers imported before selection resolve auth, legal and frontend URLs when used", async () => {
  const { api, domains } = loadResolver("https://nyx-api.chrono-ai.fun/api/v1", NEW_DOMAIN_ENV);
  const env = loadTestModule<typeof import("../env")>("../env.ts", {
    process: { env: {} },
    require: (name: string) => { assert.equal(name, "./domains"); return domains; },
  });
  const legal = loadTestModule<typeof import("../../features/legal/legalDocumentUrl")>("../../features/legal/legalDocumentUrl.ts", {
    require: (name: string) => { assert.equal(name, "../../lib/env"); return env; },
  });
  const requests: string[] = [];
  const { http, mobileApi } = loadApiConsumers(api, async (input) => {
    requests.push(String(input));
    return new Response(JSON.stringify(String(input).endsWith("/auth/refresh")
      ? { access_token: "fresh", refresh_token: "next", expires_in: 3600 }
      : { user_id: "test-user" }), { status: requests.length === 1 ? 401 : 200 });
  });

  for (const read of [api.getApiBaseUrl, env.getFrontendUrl, env.getLegalBaseUrl, env.firstPartyFrontendUrls,
    () => legal.resolveLegalDocumentUrl("privacy"),
    () => mobileApi.getSocialAuthorizeUrl("google", "nyxid://auth/social/callback")]) {
    assert.throws(read, /Domains have not been selected/);
  }
  assert.deepEqual(requests, []);
  for (const enabled of [true, false, true]) {
    domains.selectNewDomains(enabled);
    const selected = enabled ? domains.DOMAIN_SETS.new! : domains.DOMAIN_SETS.legacy;
    assert.equal(api.buildApiUrl("/auth/login"), `${selected.apiBaseUrl}/auth/login`);
    for (const provider of ["google", "github", "apple"] as const) {
      const social = new URL(mobileApi.getSocialAuthorizeUrl(provider, "nyxid://auth/social/callback"));
      assert.equal(`${social.origin}${social.pathname}`, `${selected.apiBaseUrl}/auth/social/${provider}`);
      assert.equal(social.searchParams.get("client"), "mobile");
      assert.equal(social.searchParams.get("redirect_uri"), "nyxid://auth/social/callback");
    }
    for (const key of ["privacy", "terms"] as const)
      assert.equal(legal.resolveLegalDocumentUrl(key), `${selected.legalBaseUrl}/legal/${key}.md`);
    assert.equal(env.firstPartyFrontendUrls()[0], selected.frontendUrl);
    assert.equal(env.NYXID_TRUSTED_API_BASE_URLS.length, 2);
    assert.equal(env.NYXID_TRUSTED_FRONTEND_URLS.length, 2);
  }
  await http.requestJson("/users/me");
  assert.deepEqual(requests, ["users/me", "auth/refresh", "users/me"].map((path) => `https://api.nyxid.ai/api/v1/${path}`));
  await mobileApi.loginWithPassword({ email: "review@example.test", password: "test-password" });
  assert.equal(requests.at(-1), "https://api.nyxid.ai/api/v1/auth/login");
});

test("builds without a complete new set ignore the flag", () => {
  const { api, domains } = loadResolver("https://nyx-api.chrono-ai.fun/api/v1", {
    ...NEW_DOMAIN_ENV,
    EXPO_PUBLIC_NEW_LEGAL_BASE_URL: "",
  });
  assert.equal(domains.DOMAIN_SETS.new, null);
  assert.equal(domains.CONFIGURED_DOMAIN_SETS.length, 1);
  domains.selectNewDomains(true);
  assert.equal(api.getApiBaseUrl(), "https://nyx-api.chrono-ai.fun/api/v1");
  assert.equal(domains.activeDomains(), domains.DOMAIN_SETS.legacy);
});

test("endpoint inputs cannot select a different host or scheme", () => {
  const { api } = loadResolver("https://api.nyxid.ai/api/v1");
  for (const path of ["https://untrusted.example/api", "http://untrusted.example", "//untrusted.example", "nyxid://login"]) {
    assert.throws(() => api.buildApiUrl(path), /API endpoints must be relative paths/);
  }
});

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
    await http.requestAuthenticatedResponse("/assistant/nyxagent/conversations");
    assert.equal(requests[3]?.url, `${base}/assistant/nyxagent/conversations`);
    assert.equal(requests[3]?.authorization, "Bearer access-after-refresh");
  });
}
