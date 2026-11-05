import assert from "node:assert/strict";
import test from "node:test";
import { dictationLocale } from "./dictationLocale";

const IOS = [
  "de-AT",
  "de-DE",
  "en-AU",
  "en-GB",
  "en-SG",
  "en-US",
  "fr-FR",
  "ja-JP",
  "yue-CN",
  "zh-CN",
  "zh-HK",
  "zh-TW",
];

test("dictation uses the phone's language when the recognizer has it", () => {
  assert.equal(dictationLocale("en-SG", IOS), "en-SG");
  assert.equal(dictationLocale("en_GB", IOS), "en-GB");
  assert.equal(dictationLocale("DE-at", IOS), "de-AT");
});

test("script-tagged Chinese resolves to the region that writes it", () => {
  assert.equal(dictationLocale("zh-Hans-CN", IOS), "zh-CN");
  assert.equal(dictationLocale("zh-Hans-SG", IOS), "zh-CN");
  assert.equal(dictationLocale("zh-Hant-TW", IOS), "zh-TW");
  assert.equal(dictationLocale("zh-Hant-HK", IOS), "zh-HK");
  assert.equal(dictationLocale("zh-Hant", IOS), "zh-TW");
  assert.equal(dictationLocale("zh-Hant-CN", IOS), "zh-TW");
});

test("an unlisted region falls back to the language's home region, then any", () => {
  assert.equal(dictationLocale("de-CH", IOS), "de-DE");
  assert.equal(dictationLocale("fr-CA", IOS), "fr-FR");
  assert.equal(dictationLocale("en-IN", IOS), "en-US");
  assert.equal(dictationLocale("ja", IOS), "ja-JP");
});

test("an unsupported or missing language leaves the recognizer default", () => {
  assert.equal(dictationLocale("xx-YY", IOS), undefined);
  assert.equal(dictationLocale(undefined, IOS), undefined);
  assert.equal(dictationLocale("", IOS), undefined);
  assert.equal(dictationLocale("en-US", []), undefined);
});
