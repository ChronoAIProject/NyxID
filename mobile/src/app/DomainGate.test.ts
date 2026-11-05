import assert from "node:assert/strict";
import test from "node:test";
import { fileURLToPath, URL as NodeURL } from "node:url";
import { hookHarness, deferred } from "../features/assistant/assistantHookTestHarness";
import { loadTestModule } from "../lib/api/apiUrlTestHarness";
import type { DomainBootstrapDeps } from "../lib/domainBootstrap";

function domainModule(dual = true) {
  return loadTestModule<typeof import("../lib/domains")>("../domains.ts", {
    process: { env: {
      EXPO_PUBLIC_API_BASE_URL: "https://old-api.example/api/v1",
      EXPO_PUBLIC_FRONTEND_URL: "https://old.example",
      EXPO_PUBLIC_LEGAL_BASE_URL: "https://old.example",
      ...(dual ? {
        EXPO_PUBLIC_NEW_API_BASE_URL: "https://api.nyxid.ai/api/v1",
        EXPO_PUBLIC_NEW_FRONTEND_URL: "https://nyxid.ai",
        EXPO_PUBLIC_NEW_LEGAL_BASE_URL: "https://nyxid.ai",
      } : {}),
    } },
  });
}

function mountWithBootstrap(deps: DomainBootstrapDeps) {
  const domains = domainModule();
  const bootstrap = loadTestModule<typeof import("../lib/domainBootstrap")>("../domainBootstrap.ts", {
    require: (name: string) => name === "./domains" ? domains : {},
    setTimeout, clearTimeout, AbortController,
  });
  const h = hookHarness<unknown>({
    "../lib/domains": domains,
    "../lib/domainBootstrap": { initializeDomains: () => bootstrap.initializeDomains(deps) },
  });
  const { DomainGate } = h.load(fileURLToPath(new NodeURL("./DomainGate.tsx", import.meta.url)));
  const child = { app: true };
  h.mount(() => (DomainGate as (props: unknown) => unknown)({ children: child }));
  return { h, domains, child };
}

for (const unmountEarly of [false, true]) {
  test(`the app cannot mount or build active URLs during domain bootstrap (unmount=${unmountEarly})`, async () => {
    const domains = domainModule();
    const reply = deferred<Response>();
    const calls: string[] = [];
    const bootstrap = loadTestModule<typeof import("../lib/domainBootstrap")>("../domainBootstrap.ts", {
      require: (name: string) => name === "./domains" ? domains : {},
      setTimeout, clearTimeout, AbortController,
    });
    const h = hookHarness<unknown>({
      "../lib/domains": domains,
      "../lib/domainBootstrap": {
        initializeDomains: () => bootstrap.initializeDomains({
          fetch: async (url) => { calls.push(String(url)); return reply.promise; },
          getItem: async () => null,
          setItem: async () => {},
        }),
      },
    });
    const { DomainGate } = h.load(fileURLToPath(new NodeURL("./DomainGate.tsx", import.meta.url)));
    const child = { app: true };
    h.mount(() => (DomainGate as (props: unknown) => unknown)({ children: child }));
    assert.equal(h.value(), null);
    await h.settle();
    assert.equal(h.value(), null);
    assert.equal(calls.length, 2, "only the two embedded bootstrap hosts are contacted");
    assert.throws(domains.activeDomains, /Domains have not been selected/);
    if (unmountEarly) h.unmount();
    reply.resolve(new Response(JSON.stringify({ mobile_new_domains: true })));
    await h.settle();
    assert.equal(h.value(), unmountEarly ? null : child);
    assert.equal(domains.activeDomains().apiBaseUrl, "https://api.nyxid.ai/api/v1");
    h.unmount();
  });
}

test("single-set builds render immediately without a flag lookup", () => {
  const h = hookHarness<unknown>({
    "../lib/domains": domainModule(false),
    "../lib/domainBootstrap": { initializeDomains: () => assert.fail("single-set network lookup") },
  });
  const { DomainGate } = h.load(fileURLToPath(new NodeURL("./DomainGate.tsx", import.meta.url)));
  const child = { app: true };
  h.mount(() => (DomainGate as (props: unknown) => unknown)({ children: child }));
  assert.equal(h.value(), child);
  h.unmount();
});

for (const { dependency, remote, saved, expectedNew } of [
  { dependency: "getItem", remote: true, saved: null, expectedNew: true },
  { dependency: "getItem", remote: false, saved: null, expectedNew: false },
  { dependency: "setItem", remote: true, saved: "false", expectedNew: true },
  { dependency: "setItem", remote: false, saved: "true", expectedNew: false },
  { dependency: "getItem", remote: undefined, saved: null, expectedNew: false },
  { dependency: "fetch", remote: undefined, saved: "true", expectedNew: true },
] as const) {
  for (const failure of ["throw", "reject"] as const) {
    test(`remote=${remote ?? "unavailable"}, saved=${saved}, ${dependency} ${failure} selects ${expectedNew ? "new" : "legacy"} and opens the gate`, async (t) => {
      let failures = 0;
      const calls: string[] = [];
      const writes: string[] = [];
      const fail = (): Promise<never> => {
        failures++;
        if (failure === "throw") throw new Error("synchronous dependency failure");
        return Promise.reject(new Error("rejected dependency"));
      };
      const { h, domains, child } = mountWithBootstrap({
        fetch: (url) => {
          calls.push(String(url));
          return dependency === "fetch"
            ? fail()
            : Promise.resolve(new Response(JSON.stringify({ mobile_new_domains: remote })));
        },
        getItem: () => dependency === "getItem" ? fail() : Promise.resolve(saved),
        setItem: (_key, value) => {
          writes.push(value);
          return dependency === "setItem" ? fail() : Promise.resolve();
        },
      });
      t.after(h.unmount);
      assert.equal(h.value(), null);
      assert.throws(domains.activeDomains, /Domains have not been selected/);
      await h.settle();
      assert.equal(h.value(), child, "bootstrap failures must release the splash");
      assert.equal(domains.activeDomains(), expectedNew ? domains.DOMAIN_SETS.new : domains.DOMAIN_SETS.legacy);
      assert.equal(failures, dependency === "fetch" ? 2 : 1, "each injected failure must be exercised");
      assert.deepEqual(calls.sort(), [
        "https://old-api.example/api/v1/public/config",
        "https://api.nyxid.ai/api/v1/public/config",
      ].sort());
      assert.deepEqual(writes, remote === undefined ? [] : [String(remote)]);
    });
  }
}

test("an unavailable remote uses saved true and opens the gate without a storage write", async (t) => {
  const { h, domains, child } = mountWithBootstrap({
    fetch: async () => new Response(JSON.stringify({})),
    getItem: async () => "true",
    setItem: async () => assert.fail("an unavailable remote must not overwrite the saved answer"),
  });
  t.after(h.unmount);
  assert.equal(h.value(), null);
  assert.throws(domains.activeDomains, /Domains have not been selected/);
  await h.settle();
  assert.equal(h.value(), child);
  assert.equal(domains.activeDomains(), domains.DOMAIN_SETS.new);
});

for (const saved of [null, "false", "true"]) {
  test(`an unexpected bootstrap throw waits for the saved fallback and opens the gate (saved=${saved})`, async (t) => {
    const local = deferred<string | null>();
    const { h, domains, child } = mountWithBootstrap({
      fetch: async () => assert.fail("setup failed before fetch"),
      getItem: () => local.promise,
      setItem: async () => {},
      get timeoutMs(): number { throw new Error("unexpected bootstrap setup failure"); },
    });
    t.after(h.unmount);
    await h.settle();
    assert.equal(h.value(), null);
    assert.throws(domains.activeDomains, /Domains have not been selected/);
    local.resolve(saved);
    await h.settle();
    assert.equal(h.value(), child);
    assert.equal(domains.activeDomains(), saved === "true" ? domains.DOMAIN_SETS.new : domains.DOMAIN_SETS.legacy);
  });
}

test("pre-gate notification setup and headless delivery persist signals without an API request", async () => {
  let handler: any;
  let task: any;
  const stored: string[] = [];
  const h = hookHarness<unknown>({
    "expo-notifications": { setNotificationHandler: (value: unknown) => { handler = value; } },
    "expo-secure-store": { setItemAsync: async (_key: string, value: string) => { stored.push(value); } },
    "expo-task-manager": {
      isTaskDefined: () => false,
      defineTask: (_name: string, callback: unknown) => { task = callback; },
    },
    "react-native": { AppState: { currentState: "background" }, Platform: { OS: "ios" } },
    "../api/mobileApi": { mobileApi: new Proxy({}, { get: () => assert.fail("pre-gate API access") }) },
  });
  const module = h.load(fileURLToPath(new NodeURL("../lib/notifications/pushNotifications.ts", import.meta.url)));
  (module.bootstrapNotificationInfrastructure as () => void)();
  assert.ok(handler);
  await task({ data: { data: { type: "approval_request", request_id: "request", challenge_id: "challenge" } } });
  assert.equal(stored.length, 1);
  assert.equal(JSON.parse(stored[0]!).challenge_id, "challenge");
});
