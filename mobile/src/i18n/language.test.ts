import assert from "node:assert/strict";
import test from "node:test";
import { URL, fileURLToPath } from "node:url";
import {
  initialLanguage,
  LANGUAGES,
  LANGUAGE_STORAGE_KEY,
  resolveLanguage,
} from "./language";
import { translate } from "./translate";
import {
  deferred,
  hookHarness,
} from "../features/assistant/assistantHookTestHarness";
import cn from "./locales/zh-CN.json";
import tw from "./locales/zh-TW.json";

test("native language negotiation matches main, including script priority over region", () => {
  for (const [tag, expected] of Object.entries({
    en: "en",
    "en-GB": "en",
    zh: "zh-CN",
    "zh-SG": "zh-CN",
    zh_CN: "zh-CN",
    "zh-HK": "zh-TW",
    "zh-MO": "zh-TW",
    "zh-TW": "zh-TW",
    "zh-Hans-TW": "zh-CN",
    "zh-Hant-CN": "zh-TW",
    "zh-Latn-TW": "zh-CN",
  }))
    assert.equal(resolveLanguage(tag), expected, tag);
  assert.equal(resolveLanguage("invalid locale"), undefined);
  assert.equal(resolveLanguage("fr"), undefined);
  assert.equal(initialLanguage("zh-TW", ["en"]), "zh-TW");
  assert.equal(
    initialLanguage("unsupported", ["fr", "zh-Hant-HK", "en"]),
    "zh-TW",
  );
  assert.equal(initialLanguage(null, ["fr"]), "en");
});

type Element = { type: unknown; props: Record<string, any> };
test("saved Chinese preferences restore on Hermes without Intl.Locale", () => {
  const h = hookHarness({}, { Intl: {} });
  const native = h.load(
    fileURLToPath(new URL("./language.ts", import.meta.url)),
  ) as {
    initialLanguage: typeof initialLanguage;
  };
  assert.equal(native.initialLanguage("zh-CN", ["en-US"]), "zh-CN");
  assert.equal(native.initialLanguage("zh-TW", ["en-US"]), "zh-TW");
  assert.equal(native.initialLanguage(null, ["zh-Hant-CN"]), "zh-TW");
});

function setup(
  saved: Promise<string | null> = Promise.resolve(null),
  locales = ["en-US"],
) {
  const writes: Array<{ key: string; value: string }> = [];
  let response: Promise<void> | undefined;
  const children = { draft: "我的草稿 / unchanged draft" };
  const h = hookHarness<Element | null>({
    "react/jsx-runtime": {
      jsx: (type: unknown, props: Element["props"]) => ({ type, props }),
    },
    "react-native": {
      NativeModules: {
        SettingsManager: { settings: { AppleLanguages: locales } },
      },
    },
    "@react-native-async-storage/async-storage": {
      getItem: (key: string) => {
        assert.equal(key, LANGUAGE_STORAGE_KEY);
        return saved;
      },
      setItem: (key: string, value: string) => {
        writes.push({ key, value });
        return response ?? Promise.resolve();
      },
    },
  });
  const { LanguageProvider } = h.load(
    fileURLToPath(new URL("./LanguageProvider.tsx", import.meta.url)),
  ) as {
    LanguageProvider(props: { children: unknown }): Element | null;
  };
  h.mount(() => LanguageProvider({ children }));
  return {
    h,
    writes,
    children,
    current: () => h.value()!.props.value,
    persistWith: (promise: Promise<void>) => {
      response = promise;
    },
  };
}

test("saved language hydrates before the navigator mounts, then switches without replacing children", async () => {
  const saved = deferred<string | null>();
  const f = setup(saved.promise);
  assert.equal(f.h.value(), null);
  saved.resolve("zh-TW");
  await f.h.settle();
  assert.equal(f.current().language, "zh-TW");
  assert.equal(f.current().t("Settings"), tw.Settings);
  assert.equal(f.h.value()!.props.children, f.children);
  f.current().choose("zh-CN");
  f.h.flush();
  assert.equal(f.current().t("Settings"), cn.Settings);
  assert.equal(f.h.value()!.props.children, f.children);
  assert.equal(f.children.draft, "我的草稿 / unchanged draft");
  await f.h.settle();
  assert.deepEqual(f.writes, [{ key: LANGUAGE_STORAGE_KEY, value: "zh-CN" }]);
  f.h.unmount();
});

test("unavailable storage still permits language choice and retry persists it", async () => {
  const f = setup(Promise.reject(new Error("read unavailable")), [
    "zh-Hant-TW",
  ]);
  await f.h.settle();
  assert.equal(f.current().language, "zh-TW");
  const failed = deferred<void>();
  f.persistWith(failed.promise);
  f.current().choose("zh-CN");
  await f.h.settle();
  failed.reject(new Error("write unavailable"));
  await f.h.settle();
  assert.equal(f.current().language, "zh-CN");
  assert.equal(f.current().persistenceError, true);
  f.persistWith(Promise.resolve());
  f.current().retryPersistence();
  await f.h.settle();
  assert.equal(f.current().persistenceError, false);
  assert.equal(f.writes.at(-1)?.value, "zh-CN");
  f.h.unmount();
});

test("rapid switches serialize writes so the latest choice survives relaunch", async () => {
  const f = setup();
  await f.h.settle();
  const slow = deferred<void>();
  f.persistWith(slow.promise);
  f.current().choose("zh-CN");
  await f.h.settle();
  f.current().choose("zh-TW");
  await f.h.settle();
  assert.equal(f.current().language, "zh-TW");
  assert.equal(f.writes.length, 1);
  f.persistWith(Promise.resolve());
  slow.resolve();
  await f.h.settle();
  assert.deepEqual(
    f.writes.map(({ value }) => value),
    ["zh-CN", "zh-TW"],
  );
  f.h.unmount();
});

test("catalogs cover both languages and preserve interpolation and unknown content", () => {
  assert.deepEqual(Object.keys(cn).sort(), Object.keys(tw).sort());
  const placeholders = (s: string) => s.match(/\{\{\w+\}\}/g)?.sort() ?? [];
  for (const [phrase, translation] of Object.entries(cn)) {
    assert.deepEqual(placeholders(translation), placeholders(phrase), phrase);
    assert.deepEqual(
      placeholders(tw[phrase as keyof typeof tw]),
      placeholders(phrase),
      phrase,
    );
  }
  for (const { code } of LANGUAGES) {
    assert.equal(
      translate(code, "User-authored message {{name}}", { name: "Alice 设置" }),
      "User-authored message Alice 设置",
    );
    assert.equal(translate(code, "Marin"), "Marin");
    assert.equal(
      translate(code, "https://example.com/secret?mode=voice"),
      "https://example.com/secret?mode=voice",
    );
  }
  assert.equal(
    translate("zh-CN", "Reset to {{value}} pt", { value: 120 }),
    "重置为 120 pt",
  );
});
