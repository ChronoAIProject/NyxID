import assert from "node:assert/strict";
import test from "node:test";
import { messageTime } from "./messageTime";

test("the selected app language controls dates even when the device language differs", async () => {
  const { setAppTranslationLanguage } = await import(
    "../../i18n/appTranslation"
  );
  try {
    setAppTranslationLanguage("zh-CN");
    const cn = messageTime("2026-11-14T08:10:00Z", {
      timeZone: "UTC",
      now: new Date("2026-11-14"),
    });
    assert.match(cn!.label, /11月/);
    setAppTranslationLanguage("zh-TW");
    const tw = messageTime("2026-11-14T08:10:00Z", {
      timeZone: "UTC",
      now: new Date("2026-11-14"),
    });
    assert.match(tw!.label, /11月/);
    assert.match(
      messageTime("2026-11-14T08:10:00Z", { locale: "en-US", timeZone: "UTC" })!
        .label,
      /Nov/,
    );
  } finally {
    setAppTranslationLanguage("en");
  }
});
