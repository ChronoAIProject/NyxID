import assert from "node:assert/strict";
import { readdirSync, readFileSync } from "node:fs";
import { join, relative } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";
import type { WebBrowserCustomTabsResults, WebBrowserOpenOptions } from "expo-web-browser";
import { deferred, hookHarness } from "../features/assistant/assistantHookTestHarness";

const browsers: WebBrowserCustomTabsResults = {
  preferredBrowserPackage: "preferred.browser",
  defaultBrowserPackage: "default.browser",
  browserPackages: ["first.browser", "second.browser"],
  servicePackages: [],
};

function setup(
  platform: "android" | "ios" = "android",
  lookup: () => Promise<WebBrowserCustomTabsResults | undefined> = async () => browsers,
) {
  let lookups = 0;
  const opens: unknown[][] = [];
  const authOpens: unknown[][] = [];
  const webBrowser = {
    getCustomTabsSupportingBrowsersAsync: () => {
      lookups++;
      return lookup();
    },
    openBrowserAsync: (...args: unknown[]) => {
      opens.push(args);
      return Promise.resolve({ type: "dismiss" });
    },
    openAuthSessionAsync: (...args: unknown[]) => {
      authOpens.push(args);
      return Promise.resolve({ type: "dismiss" });
    },
  };
  const h = hookHarness({
    "react-native": { Platform: { OS: platform } },
    "expo-web-browser": webBrowser,
  });
  const helper = h.load(fileURLToPath(new URL("./inAppBrowser.ts", import.meta.url))) as
    typeof import("./inAppBrowser");
  return { ...helper, authOpens, h, lookups: () => lookups, opens, webBrowser };
}

function localizeCalls(calls: unknown[][]) {
  return calls.map((args) =>
    args.map((arg) =>
      arg && typeof arg === "object" && !Array.isArray(arg)
        ? Object.fromEntries(Object.entries(arg))
        : arg,
    ),
  );
}

for (const [label, result, expected] of [
  ["preferred", browsers, "preferred.browser"],
  ["default", { ...browsers, preferredBrowserPackage: undefined }, "default.browser"],
  ["first", { ...browsers, preferredBrowserPackage: undefined, defaultBrowserPackage: undefined }, "first.browser"],
] as const) {
  test(`Android selects the ${label} Custom Tabs browser and preserves caller options`, async () => {
    const f = setup("android", async () => result);
    const extra = { createTask: false, showInRecents: true, preferEphemeralSession: true };
    const options = await f.inAppBrowserOptions(extra);
    assert.deepEqual({ ...options }, { ...extra, browserPackage: expected });
    assert.deepEqual(extra, { createTask: false, showInRecents: true, preferEphemeralSession: true });
    await f.openInAppBrowser("https://nyx.example/account/unknown", extra);
    assert.deepEqual(f.opens, [["https://nyx.example/account/unknown", options]]);
    assert.equal(f.lookups(), 1);
  });
}

test("concurrent and subsequent Android opens share one process lookup", async () => {
  const pending = deferred<WebBrowserCustomTabsResults>();
  const f = setup("android", () => pending.promise);
  const first = f.inAppBrowserOptions();
  const second = f.openInAppBrowser("https://provider.example/authorize");
  assert.equal(f.lookups(), 1);
  pending.resolve(browsers);
  assert.deepEqual({ ...await first }, { browserPackage: "preferred.browser" });
  await second;
  await f.inAppBrowserOptions();
  assert.equal(f.lookups(), 1);
});

for (const kind of ["rejects", "throws", "empty", "missing"] as const) {
  test(`an unavailable Android lookup (${kind}) is not cached`, async () => {
    let available = false;
    const f = setup("android", () => {
      if (available) return Promise.resolve(browsers);
      if (kind === "throws") throw new Error("unavailable");
      if (kind === "rejects") return Promise.reject(new Error("unavailable"));
      return Promise.resolve(kind === "missing" ? undefined : {
        browserPackages: [], servicePackages: [],
      });
    });
    const extra = { createTask: false, showInRecents: true };
    assert.equal(await f.inAppBrowserOptions(extra), extra);
    assert.equal(await f.inAppBrowserOptions(), undefined);
    assert.equal(f.lookups(), 2);
    await f.openInAppBrowser("https://nyx.example/account/unknown");
    await f.openInAppBrowser("https://nyx.example/billing", extra);
    assert.deepEqual(f.opens, [
      ["https://nyx.example/account/unknown"],
      ["https://nyx.example/billing", extra],
    ]);
    assert.equal(f.lookups(), 4);
    available = true;
    assert.deepEqual(
      { ...(await f.inAppBrowserOptions(extra)) },
      { ...extra, browserPackage: "preferred.browser" },
    );
    assert.equal(f.lookups(), 5);
  });
}

test("an Android browser rejection clears the package cache and retries once without it", async () => {
  const f = setup("android");
  const calls: unknown[][] = [];
  let first = true;
  await f.inAppBrowserOptions();
  f.webBrowser.openBrowserAsync = (...args: unknown[]) => {
    calls.push(args);
    if (first) {
      first = false;
      return Promise.reject(new Error("No matching activity"));
    }
    return Promise.resolve({ type: "dismiss" });
  };
  await f.openInAppBrowser("https://nyx.example/account/unknown");
  assert.deepEqual(localizeCalls(calls), [
    ["https://nyx.example/account/unknown", { browserPackage: "preferred.browser" }],
    ["https://nyx.example/account/unknown"],
  ]);
  await f.inAppBrowserOptions();
  assert.equal(f.lookups(), 2);
});

test("an Android browser rejection without a package propagates without retry", async () => {
  const f = setup("android", async () => undefined);
  const calls: unknown[][] = [];
  f.webBrowser.openBrowserAsync = (...args: unknown[]) => {
    calls.push(args);
    return Promise.reject(new Error("No matching activity"));
  };
  await assert.rejects(
    f.openInAppBrowser("https://nyx.example/account/unknown"),
    /No matching activity/,
  );
  assert.deepEqual(calls, [["https://nyx.example/account/unknown"]]);
  assert.equal(f.lookups(), 1);
});

test("an Android auth-session rejection retries once without the package", async () => {
  const f = setup("android");
  const calls: unknown[][] = [];
  let first = true;
  await f.inAppBrowserOptions();
  f.webBrowser.openAuthSessionAsync = (...args: unknown[]) => {
    calls.push(args);
    if (first) {
      first = false;
      return Promise.reject(new Error("No matching activity"));
    }
    return Promise.resolve({ type: "dismiss" });
  };
  await f.openAuthSessionInAppBrowser(
    "https://provider.example/authorize",
    "nyxid://services/oauth-return",
    { preferEphemeralSession: true },
  );
  assert.deepEqual(localizeCalls(calls), [
    [
      "https://provider.example/authorize",
      "nyxid://services/oauth-return",
      { preferEphemeralSession: true, browserPackage: "preferred.browser" },
    ],
    ["https://provider.example/authorize", "nyxid://services/oauth-return", { preferEphemeralSession: true }],
  ]);
  assert.equal(f.lookups(), 1);
});

test("an Android auth-session rejection without a package propagates without retry", async () => {
  const f = setup("android", async () => undefined);
  const calls: unknown[][] = [];
  f.webBrowser.openAuthSessionAsync = (...args: unknown[]) => {
    calls.push(args);
    return Promise.reject(new Error("No matching activity"));
  };
  await assert.rejects(
    f.openAuthSessionInAppBrowser(
      "https://provider.example/authorize",
      "nyxid://services/oauth-return",
    ),
    /No matching activity/,
  );
  assert.deepEqual(localizeCalls(calls), [
    ["https://provider.example/authorize", "nyxid://services/oauth-return"],
  ]);
  assert.equal(f.lookups(), 1);
});

test("the in-app browser mock resolves a WebBrowser mock registered after harness creation", async () => {
  const f = setup("android");
  const calls: unknown[][] = [];
  f.h.mock("expo-web-browser", {
    openBrowserAsync: (...args: unknown[]) => {
      calls.push(args);
      return Promise.resolve({ type: "dismiss" });
    },
  });
  const consumer = f.h.load(
    fileURLToPath(
      new URL(
        "../features/assistant/testFixtures/inAppBrowserConsumer.ts",
        import.meta.url,
      ),
    ),
  ) as { openInAppBrowserForTest(url: string): Promise<unknown> };
  await consumer.openInAppBrowserForTest("https://nyx.example/account/unknown");
  assert.deepEqual(localizeCalls(calls), [
    ["https://nyx.example/account/unknown"],
  ]);
});

test("the in-app browser mock fails loudly without an explicit WebBrowser mock", () => {
  const h = hookHarness({
    "react-native": { Platform: { OS: "ios" } },
  });
  const consumer = h.load(
    fileURLToPath(
      new URL(
        "../features/assistant/testFixtures/inAppBrowserConsumer.ts",
        import.meta.url,
      ),
    ),
  ) as { openInAppBrowserForTest(url: string): Promise<unknown> };
  assert.throws(
    () => consumer.openInAppBrowserForTest("https://nyx.example/account/unknown"),
    /inAppBrowser called without an expo-web-browser mock/,
  );
});

test("iOS keeps options, argument count and immediate browser invocation unchanged without a lookup", async () => {
  const f = setup("ios");
  const extra: WebBrowserOpenOptions = { controlsColor: "#000000" };
  assert.equal(await f.inAppBrowserOptions(extra), extra);
  assert.equal(await f.inAppBrowserOptions(), undefined);
  const first = f.openInAppBrowser("https://nyx.example/account/unknown");
  const second = f.openInAppBrowser("https://nyx.example/billing", extra);
  assert.deepEqual(f.opens, [
    ["https://nyx.example/account/unknown"],
    ["https://nyx.example/billing", extra],
  ]);
  assert.equal(f.opens[1]![1], extra);
  await Promise.all([first, second]);
  f.webBrowser.openAuthSessionAsync = (...args: unknown[]) => {
    f.authOpens.push(args);
    return Promise.reject(new Error("No matching activity"));
  };
  await assert.rejects(
    f.openAuthSessionInAppBrowser(
      "https://provider.example/authorize",
      "nyxid://services/oauth-return",
    ),
    /No matching activity/,
  );
  assert.equal(f.lookups(), 0);
  assert.deepEqual(f.authOpens, [
    ["https://provider.example/authorize", "nyxid://services/oauth-return"],
  ]);
});

test("browser opens use the helper; only social login may open an auth session directly", () => {
  const sourceRoot = fileURLToPath(new URL("../", import.meta.url));
  const directBrowserCalls: string[] = [];
  const authSessionCalls: string[] = [];
  const visit = (directory: string) => {
    for (const entry of readdirSync(directory, { withFileTypes: true })) {
      const filename = join(directory, entry.name);
      if (entry.isDirectory()) visit(filename);
      else if (/\.(ts|tsx)$/.test(entry.name) && !/\.test\.(ts|tsx)$/.test(entry.name)) {
        const source = readFileSync(filename, "utf8");
        if (/WebBrowser\.openBrowserAsync\s*\(/.test(source))
          directBrowserCalls.push(relative(sourceRoot, filename));
        if (/WebBrowser\.openAuthSessionAsync\s*\(/.test(source)) {
          authSessionCalls.push(relative(sourceRoot, filename));
        }
        if (relative(sourceRoot, filename) === "features/services/useServiceOAuth.ts")
          assert.match(source, /await openAuthSessionInAppBrowser\(/);
      }
    }
  };
  visit(sourceRoot);
  assert.deepEqual(directBrowserCalls, ["lib/inAppBrowser.ts"]);
  assert.deepEqual(authSessionCalls.sort(), [
    "features/auth/AuthHomeScreen.tsx",
    "lib/inAppBrowser.ts",
  ]);
});
