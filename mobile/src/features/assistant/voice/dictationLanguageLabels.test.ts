import assert from "node:assert/strict";
import test from "node:test";
import names from "./dictationLanguageNames.json";
import {
  languageLabel,
  languageOptions,
  matchesLanguage,
} from "./dictationLanguageLabels";

test("every bundled recognizer name matches Node ICU in all app languages and its endonym", () => {
  for (const [locale, row] of Object.entries(names)) {
    const tag = new Intl.Locale(locale);
    for (const [index, ui] of ["en", "zh-Hans", "zh-Hant"].entries()) {
      assert.deepEqual(
        row[index],
        [
          new Intl.DisplayNames([ui], { type: "language" }).of(tag.language),
          new Intl.DisplayNames([ui], { type: "region" }).of(tag.region!),
        ],
        `${locale}: ${ui}`,
      );
    }
    assert.equal(
      row[3],
      new Intl.DisplayNames([locale], { type: "language" }).of(tag.language),
    );
  }
  assert.ok(
    Object.keys(names).length < 200,
    "bounded recognizer vocabulary, not a world locale database",
  );
});
test("Hermes labels work without DisplayNames, with language-only and unknown fallbacks", () => {
  const original = Intl.DisplayNames;
  try {
    Object.defineProperty(Intl, "DisplayNames", {
      value: undefined,
      configurable: true,
    });
    assert.equal(languageLabel("ja-JP"), "Japanese (Japan)");
    assert.equal(languageLabel("zh-CN", "zh-CN"), "中文（中国）");
    assert.equal(languageLabel("ja-XX"), "Japanese");
    assert.equal(languageLabel("xx-YY"), "xx-YY");
  } finally {
    Object.defineProperty(Intl, "DisplayNames", {
      value: original,
      configurable: true,
    });
  }
});
test("search matches localized name, endonym, English name and code without case or width differences", () => {
  const japanese = languageOptions(["ja-JP"], "zh-CN")[0]!;
  for (const query of ["日语", "日本語", "Japanese", "ＪＡ－ｊｐ"])
    assert.ok(matchesLanguage(japanese, query), query);
  assert.equal(matchesLanguage(japanese, "Chinese"), false);
});

for (const [locale, endonym] of [
  ["ar-SA", "العربية"],
  ["he-IL", "עברית"],
] as const) {
  test(`${locale} isolates the endonym before its code without affecting search`, () => {
    const option = languageOptions([locale])[0]!;
    assert.equal(option.detail, `\u2068${endonym}\u2069 · ${locale}`);
    for (const query of [
      endonym,
      locale.toUpperCase(),
      `${endonym} · ${locale}`,
      option.detail,
    ]) {
      assert.ok(matchesLanguage(option, query), query);
    }
  });
}

test("Android Mandarin and Cantonese tags have bundled names in every app language", () => {
  for (const ui of ["en", "zh-CN", "zh-TW"] as const) {
    assert.equal(languageLabel("cmn-Hans-CN", ui), languageLabel("zh-CN", ui));
    assert.equal(languageLabel("cmn-Hant-TW", ui), languageLabel("zh-TW", ui));
    assert.notEqual(languageLabel("yue-Hant-HK", ui), "yue-Hant-HK");
  }
});
