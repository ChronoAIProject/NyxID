import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { URL as NodeURL } from "node:url";
import { runInNewContext } from "node:vm";
import ts from "typescript";

type DomainsModule = typeof import("./domains");
type BootstrapModule = typeof import("./domainBootstrap");

const LEGACY_API = "https://nyx-api.chrono-ai.fun/api/v1";
const NEW_API = "https://api.nyxid.ai/api/v1";

function loadModule<T>(filename: string, context: Record<string, unknown>): T {
  const source = readFileSync(new NodeURL(filename, import.meta.url), "utf8");
  const js = ts.transpileModule(source, {
    compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 },
  }).outputText;
  const exported = {};
  runInNewContext(js, { exports: exported, setTimeout, clearTimeout, AbortController, ...context });
  return exported as T;
}

function load(newSet = true) {
  const env = {
    EXPO_PUBLIC_API_BASE_URL: LEGACY_API,
    EXPO_PUBLIC_FRONTEND_URL: "https://nyx.chrono-ai.fun",
    EXPO_PUBLIC_LEGAL_BASE_URL: "https://nyx.chrono-ai.fun",
    EXPO_PUBLIC_NEW_API_BASE_URL: newSet ? NEW_API : "",
    EXPO_PUBLIC_NEW_FRONTEND_URL: newSet ? "https://nyxid.ai" : "",
    EXPO_PUBLIC_NEW_LEGAL_BASE_URL: newSet ? "https://nyxid.ai" : "",
  };
  const domains = loadModule<DomainsModule>("./domains.ts", { process: { env } });
  const bootstrap = loadModule<BootstrapModule>("./domainBootstrap.ts", {
    require: (name: string) => {
      if (name === "./domains") return domains;
      if (name === "expo-secure-store") return {};
      throw new Error(`Unexpected dependency: ${name}`);
    },
  });
  return { domains, bootstrap };
}

type Reply = { flag?: unknown; status?: number; delayMs?: number; fail?: boolean } | "hang";

function fakeFetch(replies: Record<string, Reply>, calls: string[]): typeof fetch {
  return ((input: string, init?: RequestInit) => {
    const url = String(input);
    calls.push(url);
    const reply = replies[url.replace(/\/public\/config$/, "")];
    return new Promise<Response>((resolve, reject) => {
      init?.signal?.addEventListener("abort", () => reject(new Error("aborted")));
      if (!reply || reply === "hang") return;
      setTimeout(() => {
        if (reply.fail) return reject(new Error("network"));
        const body = reply.flag === undefined ? {} : { mobile_new_domains: reply.flag };
        resolve(new Response(JSON.stringify(body), { status: reply.status ?? 200 }));
      }, reply.delayMs ?? 0);
    });
  }) as typeof fetch;
}

function store(saved: string | null = null) {
  const writes: string[] = [];
  return {
    writes,
    getItem: async () => saved,
    setItem: async (_key: string, value: string) => {
      writes.push(value);
    },
  };
}

test("asks both API hosts and applies the first valid answer", async () => {
  const { domains, bootstrap } = load();
  const calls: string[] = [];
  const saved = store();
  await bootstrap.initializeDomains({
    fetch: fakeFetch({ [LEGACY_API]: { flag: true, delayMs: 5 }, [NEW_API]: { flag: true, delayMs: 50 } }, calls),
    ...saved,
  });
  assert.deepEqual(calls.sort(), [`${LEGACY_API}/public/config`, `${NEW_API}/public/config`].sort());
  assert.equal(domains.NYXID_API_BASE_URL, NEW_API);
  assert.equal(domains.NYXID_FRONTEND_URL, "https://nyxid.ai");
  assert.deepEqual(saved.writes, ["true"]);
});

test("uses whichever host answers when the other is down or too old to know the flag", async () => {
  for (const [legacy, next] of [
    [{ fail: true }, { flag: true }],
    [{ flag: true }, "hang"],
    [{}, { flag: true }],
    [{ status: 503 }, { flag: true }],
  ] as const) {
    const { domains, bootstrap } = load();
    await bootstrap.initializeDomains({
      fetch: fakeFetch({ [LEGACY_API]: legacy, [NEW_API]: next }, []),
      ...store(),
      timeoutMs: 200,
    });
    assert.equal(domains.NYXID_API_BASE_URL, NEW_API);
  }
});

test("an explicit off answer returns to the original domains and is saved", async () => {
  const { domains, bootstrap } = load();
  const saved = store("true");
  await bootstrap.initializeDomains({
    fetch: fakeFetch({ [LEGACY_API]: { flag: false }, [NEW_API]: { flag: false } }, []),
    ...saved,
  });
  assert.equal(domains.NYXID_API_BASE_URL, LEGACY_API);
  assert.equal(domains.NYXID_USING_NEW_DOMAINS, false);
  assert.deepEqual(saved.writes, ["false"]);
});

test("with no answer in time, the saved choice applies, else the original domains", async () => {
  for (const [saved, expected] of [["true", NEW_API], ["false", LEGACY_API], [null, LEGACY_API]] as const) {
    const { domains, bootstrap } = load();
    const started = Date.now();
    const local = store(saved);
    await bootstrap.initializeDomains({
      fetch: fakeFetch({ [LEGACY_API]: "hang", [NEW_API]: "hang" }, []),
      ...local,
      timeoutMs: 50,
    });
    assert.ok(Date.now() - started < 1000, "the launch gate is bounded by the timeout");
    assert.equal(domains.NYXID_API_BASE_URL, expected);
    assert.deepEqual(local.writes, []);
  }
});

test("builds without a second domain set skip the network entirely", async () => {
  const { domains, bootstrap } = load(false);
  const calls: string[] = [];
  await bootstrap.initializeDomains({ fetch: fakeFetch({}, calls), ...store("true") });
  assert.deepEqual(calls, []);
  assert.equal(domains.NYXID_API_BASE_URL, LEGACY_API);
});
