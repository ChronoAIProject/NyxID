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
  const h = hookHarness({
    "react-native": { Platform: { OS: platform } },
    "expo-web-browser": {
      getCustomTabsSupportingBrowsersAsync: () => {
        lookups++;
        return lookup();
      },
      openBrowserAsync: (...args: unknown[]) => {
        opens.push(args);
        return Promise.resolve({ type: "dismiss" });
      },
    },
  });
  const helper = h.load(fileURLToPath(new URL("./inAppBrowser.ts", import.meta.url))) as
    typeof import("./inAppBrowser");
  return { ...helper, opens, lookups: () => lookups };
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
  test(`an unavailable Android lookup (${kind}) is cached and keeps the original arguments`, async () => {
    const f = setup("android", () => {
      if (kind === "throws") throw new Error("unavailable");
      if (kind === "rejects") return Promise.reject(new Error("unavailable"));
      return Promise.resolve(kind === "missing" ? undefined : {
        browserPackages: [], servicePackages: [],
      });
    });
    const extra = { createTask: false, showInRecents: true };
    assert.equal(await f.inAppBrowserOptions(extra), extra);
    assert.equal(await f.inAppBrowserOptions(), undefined);
    await f.openInAppBrowser("https://nyx.example/account/unknown");
    await f.openInAppBrowser("https://nyx.example/billing", extra);
    assert.deepEqual(f.opens, [
      ["https://nyx.example/account/unknown"],
      ["https://nyx.example/billing", extra],
    ]);
    assert.equal(f.lookups(), 1);
  });
}

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
  assert.equal(f.lookups(), 0);
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
          if (relative(sourceRoot, filename) !== "features/auth/AuthHomeScreen.tsx")
            assert.match(source, /await inAppBrowserOptions\(\{\s*preferEphemeralSession: true,?\s*\}\)/);
        }
      }
    }
  };
  visit(sourceRoot);
  assert.deepEqual(directBrowserCalls, ["lib/inAppBrowser.ts"]);
  assert.deepEqual(authSessionCalls.sort(), [
    "features/auth/AuthHomeScreen.tsx",
    "features/services/useServiceOAuth.ts",
  ]);
});
