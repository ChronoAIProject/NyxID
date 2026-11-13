import assert from "node:assert/strict";
import test from "node:test";
import { assistantTitleLanguage } from "./titleLanguage";
for (const locale of ["en-US", "zh-CN"]) {
  test(`title UI-language hint follows ${locale} locale`, (t) => {
    t.mock.method(Intl, "DateTimeFormat", () => ({ resolvedOptions: () => ({ locale }) }));
    assert.equal(assistantTitleLanguage(), locale);
  });
}
