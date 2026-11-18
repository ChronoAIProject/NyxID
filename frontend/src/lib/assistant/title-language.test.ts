import { afterEach, expect, it, vi } from "vitest";
import { appI18n } from "@/i18n/instance";
import { assistantTitleLanguage } from "./title-language";

afterEach(async () => {
  vi.unstubAllGlobals();
  await appI18n.changeLanguage("en");
});
it.each(["en", "zh-CN", "zh-TW"])("uses the active %s UI locale despite stale document/browser hints", async (language) => {
  await appI18n.changeLanguage(language);
  document.documentElement.lang = "ru-RU";
  vi.stubGlobal("navigator", { language: "de", languages: ["de"] });
  expect(assistantTitleLanguage()).toBe(language);
});
it("reads live UI language changes when the next turn is sent", async () => {
  await appI18n.changeLanguage("en");
  expect(assistantTitleLanguage()).toBe("en");
  await appI18n.changeLanguage("zh-TW");
  expect(assistantTitleLanguage()).toBe("zh-TW");
  expect(document.documentElement.lang).toBe("zh-TW");
});
