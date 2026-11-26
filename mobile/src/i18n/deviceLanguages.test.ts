import assert from "node:assert/strict";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { hookHarness } from "../features/assistant/assistantHookTestHarness";
import { initialLanguage } from "./language";
test("AppleLanguages overrides the bundle-dependent en-CN/en-JP Intl locale for both UI and dictation", () => {
  for (const preferred of [
    ["zh-Hans-CN", "en-US"],
    ["ja-JP"],
    ["zh-Hant-TW"],
  ]) {
    const h = hookHarness(
      {
        "react-native": {
          Platform: { OS: "ios" },
          Settings: {
            get: (key: string) =>
              key === "AppleLanguages" ? preferred : undefined,
          },
        },
      },
      {
        Intl: {
          DateTimeFormat: () => ({
            resolvedOptions: () => ({ locale: "en-CN" }),
          }),
        },
      },
    );
    const { deviceLanguages } = h.load(
      fileURLToPath(new URL("./deviceLanguages.ts", import.meta.url)),
    ) as typeof import("./deviceLanguages");
    assert.deepEqual([...deviceLanguages()], preferred);
    assert.equal(
      initialLanguage(null, deviceLanguages()),
      preferred[0]!.startsWith("zh-Hant")
        ? "zh-TW"
        : preferred[0]!.startsWith("zh")
          ? "zh-CN"
          : "en",
    );
  }
});
test("missing iOS AppleLanguages retains the Intl device fallback", () => {
  const h = hookHarness(
    {
      "react-native": {
        Platform: { OS: "ios" },
        Settings: { get: () => undefined },
      },
    },
    {
      Intl: {
        DateTimeFormat: () => ({
          resolvedOptions: () => ({ locale: "ja-JP" }),
        }),
      },
    },
  );
  const { deviceLanguages } = h.load(
    fileURLToPath(new URL("./deviceLanguages.ts", import.meta.url)),
  ) as typeof import("./deviceLanguages");
  assert.deepEqual([...deviceLanguages()], ["ja-JP"]);
});
test("Android returns the Intl locale without calling unsupported Settings.get", () => {
  let settingsCalls = 0;
  const h = hookHarness(
    {
      "react-native": {
        Platform: { OS: "android" },
        Settings: {
          get: () => {
            settingsCalls++;
            throw new Error("Android Settings.get would warn");
          },
        },
      },
    },
    {
      Intl: {
        DateTimeFormat: () => ({
          resolvedOptions: () => ({ locale: "ja-JP" }),
        }),
      },
    },
  );
  const { deviceLanguages } = h.load(
    fileURLToPath(new URL("./deviceLanguages.ts", import.meta.url)),
  ) as typeof import("./deviceLanguages");
  for (let render = 0; render < 3; render++) {
    assert.deepEqual([...deviceLanguages()], ["ja-JP"]);
    assert.deepEqual(
      [...deviceLanguages({ includeLocaleFallback: true })],
      ["ja-JP"],
    );
  }
  assert.equal(settingsCalls, 0);
});
test("LanguageProvider retains its previous Intl fallback after unsupported preferred UI languages", () => {
  const h = hookHarness(
    {
      "react-native": {
        Platform: { OS: "ios" },
        Settings: { get: () => ["ja-JP"] },
      },
    },
    {
      Intl: {
        DateTimeFormat: () => ({
          resolvedOptions: () => ({ locale: "zh-CN" }),
        }),
      },
    },
  );
  const { deviceLanguages } = h.load(
    fileURLToPath(new URL("./deviceLanguages.ts", import.meta.url)),
  ) as typeof import("./deviceLanguages");
  assert.equal(
    initialLanguage(null, deviceLanguages({ includeLocaleFallback: true })),
    "zh-CN",
  );
  assert.deepEqual(
    [...deviceLanguages()],
    ["ja-JP"],
    "Dictation must never append a bundle-dependent locale",
  );
});
