import assert from "node:assert/strict";
import test from "node:test";
import cases from "./dictation-language-cases.json";
import { resolveDictationLanguage } from "./resolveDictationLanguage";

for (const value of cases) {
  test(`shared dictation resolver: ${value.preferred.join(",")} saved ${value.saved ?? "automatic"}`, () => {
    assert.equal(
      resolveDictationLanguage(value.saved ?? undefined, value.preferred, [
        "zh-CN",
        "zh-TW",
        "ja-JP",
        "en-US",
      ]),
      value.expected ?? undefined,
    );
  });
}
