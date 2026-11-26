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

test("Mandarin aliases preserve script and Cantonese does not fall back to Simplified Mandarin", () => {
  assert.equal(dictationLocale("zh-Hans-CN", ["cmn-Hant-TW"]), undefined);
  assert.equal(dictationLocale("zh-HK", ["cmn-Hans-CN"]), undefined);
  assert.equal(dictationLocale("yue-Hant-HK", ["cmn-Hant-TW"]), "cmn-Hant-TW");
});

for (const [preferred, supported, expected] of [
  ["zh-Hans-CN", ["cmn-Hans-CN"], "cmn-Hans-CN"],
  ["cmn-Hant-TW", ["zh-TW"], "zh-TW"],
  ["zh-Hant-HK", ["cmn-Hant-TW", "yue-Hant-HK"], "yue-Hant-HK"],
  ["zh-MO", ["yue-MO", "zh-TW"], "yue-MO"],
  ["zh-HK", ["zh-TW", "zh-CN"], "zh-TW"],
  ...[
    ["iw", "he"],
    ["in", "id"],
    ["ji", "yi"],
    ["no", "nb"],
    ["tl", "fil"],
  ].flatMap(([old, modern]) => [
    [old!, [modern!], modern!],
    [modern!, [old!], old!],
  ]),
] as [string, string[], string][]) {
  test(`dictation locale alias ${preferred} preserves recognizer spelling ${expected}`, () => {
    assert.equal(dictationLocale(preferred, supported), expected);
  });
}
