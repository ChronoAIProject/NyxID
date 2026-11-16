import assert from "node:assert/strict";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { hookHarness } from "../assistantHookTestHarness";

function setup() {
  let saved: string | null = null;
  const h = hookHarness({}, { Intl: {} });
  h.mock("@react-native-async-storage/async-storage", {
    getItem: async () => saved,
    setItem: async (_key: string, value: string) => {
      saved = value;
    },
    removeItem: async () => {
      saved = null;
    },
  });
  return h.load(
    fileURLToPath(new URL("./dictationLanguage.ts", import.meta.url)),
  ) as typeof import("./dictationLanguage");
}

test("Hermes without DisplayNames shows localized names, regions and searchable native names", () => {
  const language = setup();
  assert.equal(language.languageLabel("ja-JP"), "Japanese (Japan)");
  assert.equal(language.languageLabel("de_AT", "zh-CN"), "德语 (奥地利)");
  assert.equal(
    language.languageLabel("zh-Hant-TW", "zh-TW"),
    "繁體中文 (台灣)",
  );
  const options = language.languageOptions([
    "zh-CN",
    "ja-JP",
    "ar-SA",
    "ja-JP",
  ]);
  assert.equal(options.length, 3);
  assert.equal(
    options.find((o) => o.value === "ja-JP")?.detail,
    "日本語 · ja-JP",
  );
  assert.equal(
    options.find((o) => o.value === "zh-CN")?.label,
    "Chinese (China)",
  );
  assert.equal(
    options.find((o) => o.value === "ar-SA")?.label,
    "Arabic (Saudi Arabia)",
  );
});

test("the selected dictation language persists and device language removes the override", async () => {
  const language = setup();
  assert.equal(await language.savedDictationLanguage(), undefined);
  await language.saveDictationLanguage("ja-JP");
  assert.equal(await language.savedDictationLanguage(), "ja-JP");
  await language.saveDictationLanguage(undefined);
  assert.equal(await language.savedDictationLanguage(), undefined);
  assert.equal(language.languageLabel("xx-YY"), "xx-YY");
});
