import assert from "node:assert/strict";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { hookHarness } from "../assistantHookTestHarness";
import type { Language } from "../../../i18n/language";
import { translate } from "../../../i18n/translate";

test("device language stays first, choices update and reopen using the app language", async () => {
  let saved: string | undefined;
  let language: Language = "en";
  const h = hookHarness<
    ReturnType<typeof import("./useDictationLanguage").useDictationLanguage>
  >(
    {
      "expo-speech-recognition": {
        ExpoSpeechRecognitionModule: {
          getSupportedLocales: async () => ({
            locales: ["en-US", "ja-JP", "zh-CN"],
          }),
        },
      },
      "../../../i18n/LanguageProvider": {
        useAppLanguage: () => ({
          language,
          t: (phrase: string) => translate(language, phrase),
        }),
      },
      "@react-native-async-storage/async-storage": {
        getItem: async () => saved,
        setItem: async (_key: string, value: string) => {
          saved = value;
        },
        removeItem: async () => {
          saved = undefined;
        },
      },
    },
    {
      Intl: {
        DateTimeFormat: () => ({
          resolvedOptions: () => ({ locale: "en-US" }),
        }),
      },
    },
  );
  h.mock(
    "./dictationLanguage",
    h.load(fileURLToPath(new URL("./dictationLanguage.ts", import.meta.url))),
  );
  const { useDictationLanguage } = h.load(
    fileURLToPath(new URL("./useDictationLanguage.ts", import.meta.url)),
  ) as typeof import("./useDictationLanguage");
  let active = true;
  h.mount(() => useDictationLanguage(active));
  await h.settle();
  assert.equal(h.value().options[0]?.value, "automatic");
  assert.match(h.value().options[0]!.detail!, /English.*en-US/);
  h.value().choose("ja-JP");
  await h.settle();
  assert.equal(h.value().value, "ja-JP");
  assert.equal(h.value().label, "Japanese (Japan)");
  assert.equal(saved, "ja-JP");
  language = "zh-CN";
  h.render();
  assert.equal(h.value().label, "日语 (日本)");
  active = false;
  h.render();
  saved = "zh-CN";
  active = true;
  h.render();
  await h.settle();
  assert.equal(h.value().value, "zh-CN");
  h.value().choose("automatic");
  await h.settle();
  assert.equal(saved, undefined);
  assert.equal(h.value().value, "automatic");
  assert.equal(h.value().label, "英语 (美国)");
  active = false;
  h.render();
  saved = "unsupported-ZZ";
  active = true;
  h.render();
  await h.settle();
  assert.equal(h.value().value, "automatic");
  assert.equal(h.value().label, "英语 (美国)");
  h.unmount();
});
