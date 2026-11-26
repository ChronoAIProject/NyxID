import assert from "node:assert/strict";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { deferred, hookHarness } from "../assistantHookTestHarness";
import { resolveDictationLanguage } from "./resolveDictationLanguage";
import { languageLabel, languageOptions } from "./dictationLanguageLabels";
import type { useDictationLanguage } from "./useDictationLanguage";

function setup(
  saved?: string,
  recent = ["fr-FR", "ja-JP", "en-US"],
  locales:
    | Promise<{ locales: string[] }>
    | (() => Promise<{ locales: string[] }>) = Promise.resolve({
    locales: ["en-US", "zh-CN", "ja-JP", "fr-FR"],
  }),
  platform = "ios",
) {
  const writes: (string | undefined)[] = [];
  const h = hookHarness<ReturnType<typeof useDictationLanguage>>({
    "react-native": { Platform: { OS: platform } },
    "expo-speech-recognition": {
      ExpoSpeechRecognitionModule: {
        getSupportedLocales: () =>
          typeof locales === "function" ? locales() : locales,
      },
    },
    "../../../i18n/deviceLanguages": {
      deviceLanguages: () => ["zh-Hans-CN", "en-US"],
    },
    "../../../i18n/LanguageProvider": {
      useAppLanguage: () => ({
        language: "zh-CN",
        t: (text: string, values: Record<string, unknown> = {}) =>
          text.replace(/\{\{(\w+)\}\}/g, (_, key: string) =>
            String(values[key]),
          ),
      }),
    },
    "./resolveDictationLanguage": { resolveDictationLanguage },
    "./dictationLanguage": {
      languageLabel,
      languageOptions,
      savedDictationLanguage: async () => saved,
      recentDictationLanguages: async () => recent,
      saveDictationLanguage: async (value?: string) => {
        writes.push(value);
        saved = value;
      },
      rememberDictationLanguage: async (value: string) => {
        recent = [value, ...recent.filter((x) => x !== value)].slice(0, 3);
      },
    },
  });
  h.mock(
    "./recognitionLocales",
    h.load(fileURLToPath(new URL("./recognitionLocales.ts", import.meta.url))),
  );
  const module = h.load(
    fileURLToPath(new URL("./useDictationLanguage.ts", import.meta.url)),
  ) as typeof import("./useDictationLanguage");
  h.mount(() => module.useDictationLanguage(true));
  return { h, writes };
}
test("automatic picker resolves device preferences, localizes the summary and suggests supported devices then recent choices", async () => {
  const { h } = setup();
  await h.settle();
  assert.equal(h.value().value, "automatic");
  assert.equal(h.value().resolved, "zh-CN");
  assert.equal(h.value().summary, "Device language · 中文（中国）");
  assert.deepEqual(
    [...h.value().suggested],
    ["zh-CN", "en-US", "fr-FR", "ja-JP"],
  );
  assert.equal(h.value().options[0]!.detail, "中文（中国）");
  h.unmount();
});
test("choosing persists the existing locale format, updates recent choices and automatic removes the old selection", async () => {
  const { h, writes } = setup("ja-JP");
  await h.settle();
  assert.equal(h.value().summary, "日语（日本）");
  h.value().choose("fr-FR");
  await h.settle();
  assert.equal(h.value().resolved, "fr-FR");
  assert.deepEqual(writes, ["fr-FR"]);
  h.value().choose("automatic");
  await h.settle();
  assert.equal(h.value().resolved, "zh-CN");
  assert.deepEqual(writes, ["fr-FR", undefined]);
  h.unmount();
});
test("saved but unsupported language is visibly automatic, without changing stored choice", async () => {
  const { h, writes } = setup("xx-YY");
  await h.settle();
  assert.equal(h.value().resolved, "zh-CN");
  assert.equal(h.value().value, "automatic");
  assert.deepEqual(writes, []);
  h.unmount();
});
test("recent language storage is bounded to three and leaves the original locale key format intact", async () => {
  const storage = new Map<string, string>();
  const h = hookHarness({
    "@react-native-async-storage/async-storage": {
      getItem: async (key: string) => storage.get(key) ?? null,
      setItem: async (key: string, value: string) => {
        storage.set(key, value);
      },
      removeItem: async (key: string) => {
        storage.delete(key);
      },
    },
  });
  const data = h.load(
    fileURLToPath(new URL("./dictationLanguage.ts", import.meta.url)),
  ) as typeof import("./dictationLanguage");
  await data.saveDictationLanguage("ja-JP");
  assert.equal(storage.get("nyxid.dictation_language"), "ja-JP");
  for (const locale of ["en-US", "zh-CN", "ja-JP", "fr-FR", "zh-CN"])
    await data.rememberDictationLanguage(locale);
  assert.deepEqual(
    [...(await data.recentDictationLanguages())],
    ["zh-CN", "fr-FR", "ja-JP"],
  );
  await data.saveDictationLanguage(undefined);
  assert.equal(storage.has("nyxid.dictation_language"), false);
});

test("locale loading preserves the saved label and does not claim no device language", async () => {
  const locales = deferred<{ locales: string[] }>();
  const { h } = setup("ja-JP", [], locales.promise);
  await h.settle();
  assert.equal(h.value().loading, true);
  assert.equal(h.value().summary, "日语（日本）");
  assert.equal(h.value().value, "ja-JP");
  assert.equal(h.value().options[0]!.detail, undefined);
  locales.resolve({ locales: [] });
  await h.settle();
  assert.equal(h.value().loading, false);
  assert.equal(h.value().value, "ja-JP");
  assert.equal(h.value().resolved, "ja-JP");
  assert.equal(h.value().listUnavailable, true);
  assert.equal(h.value().options[0]!.detail, "中文（中国）");
  h.unmount();
});
test("automatic language does not show an unsupported warning while locales load", async () => {
  const locales = deferred<{ locales: string[] }>();
  const { h } = setup(undefined, [], locales.promise);
  await h.settle();
  assert.equal(h.value().loading, true);
  assert.equal(h.value().options[0]!.detail, undefined);
  locales.resolve({ locales: ["zh-CN"] });
  await h.settle();
  assert.equal(h.value().resolved, "zh-CN");
  h.unmount();
});

for (const rejected of [false, true]) {
  test(`Android discovery ${rejected ? "rejection" : "empty list"} settles with a saved language and a clear explanation`, async () => {
    const list = deferred<{ locales: string[] }>();
    const { h } = setup("cmn-Hans-CN", [], list.promise, "android");
    if (rejected) list.reject(new Error("API unavailable"));
    else list.resolve({ locales: [] });
    await h.settle();
    assert.equal(h.value().loading, false);
    assert.equal(h.value().listUnavailable, true);
    assert.equal(h.value().value, "cmn-Hans-CN");
    assert.equal(h.value().resolved, "cmn-Hans-CN");
    assert.equal(h.value().label, "中文（中国）");
    assert.match(h.value().explanation!, /did not provide a language list/);
    h.unmount();
  });
}

test("Android discovery times out at 2.5s and Retry re-queries the provider", async (t) => {
  t.mock.timers.enable({ apis: ["setTimeout"] });
  let queries = 0;
  const { h } = setup(
    undefined,
    [],
    () => {
      queries++;
      return queries === 1
        ? new Promise(() => {})
        : Promise.resolve({ locales: ["zh-CN"] });
    },
    "android",
  );
  await h.settle();
  t.mock.timers.tick(2499);
  await h.settle();
  assert.equal(h.value().loading, true);
  t.mock.timers.tick(1);
  await h.settle();
  assert.equal(h.value().loading, false);
  assert.equal(h.value().listUnavailable, true);
  assert.equal(h.value().manualEntry, true);
  h.value().retry();
  await h.settle();
  assert.equal(queries, 2);
  assert.equal(h.value().listUnavailable, false);
  assert.equal(h.value().resolved, "zh-CN");
  assert.deepEqual(
    [...h.value().options].map((o) => o.value),
    ["automatic", "zh-CN"],
  );
  h.unmount();
});
