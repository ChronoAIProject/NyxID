import { afterEach, describe, expect, it } from "vitest";
import { appI18n, translate } from ".";
import en from "./locales/en.json";
import zhCN from "./locales/zh-CN.json";
import zhTW from "./locales/zh-TW.json";
import onboardingEn from "@/features/nyxbot-onboarding/locales/en.json";
import onboardingZhCN from "@/features/nyxbot-onboarding/locales/zh-CN.json";
import onboardingZhTW from "@/features/nyxbot-onboarding/locales/zh-TW.json";
import wizardEn from "./locales/wizard-en.json";
import wizardZhCN from "./locales/wizard-zh-CN.json";
import wizardZhTW from "./locales/wizard-zh-TW.json";
import commonEn from "./locales/common-en.json";
import commonZhCN from "./locales/common-zh-CN.json";
import commonZhTW from "./locales/common-zh-TW.json";
import errorsEn from "./locales/errors-en.json";
import errorsZhCN from "./locales/errors-zh-CN.json";
import errorsZhTW from "./locales/errors-zh-TW.json";
import { buildStudioBreadcrumbs } from "@/lib/studio-breadcrumbs";
import { LANGUAGE_STORAGE_KEY } from "./language";
import { formatBucketDate } from "@/lib/usage-bucket";
import { formatEstimatedCredits } from "@/lib/billing-format";

afterEach(async () => {
  await appI18n.changeLanguage("en");
});
const placeholders = (value: string) =>
  [...value.matchAll(/{{\s*([^}]+?)\s*}}/g)].map((match) => match[1]).sort();

describe("translation catalogs", () => {
  it.each([
    ["Simplified Chinese", en, zhCN],
    ["Simplified Chinese wizard", wizardEn, wizardZhCN],
    ["Traditional Chinese wizard", wizardEn, wizardZhTW],
    ["Simplified Chinese common", commonEn, commonZhCN],
    ["Traditional Chinese common", commonEn, commonZhTW],
    ["Simplified Chinese errors", errorsEn, errorsZhCN],
    ["Traditional Chinese errors", errorsEn, errorsZhTW],
    ["Traditional Chinese", en, zhTW],
    ["Simplified Chinese onboarding", onboardingEn, onboardingZhCN],
    ["Traditional Chinese onboarding", onboardingEn, onboardingZhTW],
  ])(
    "has complete keys and interpolation values for %s",
    (_label, english, translations) => {
      expect(Object.keys(translations).sort()).toEqual(
        Object.keys(english).sort(),
      );
      for (const [key, value] of Object.entries(english)) {
        const translated = (translations as Record<string, string>)[key]!;
        expect(translated.trim(), key).not.toBe("");
        expect(placeholders(translated), key).toEqual(placeholders(value));
      }
    },
  );

  it("interpolates names without changing user content or treating it as markup", async () => {
    await appI18n.changeLanguage("zh-TW");
    expect(
      translate("Welcome back, {{name}}", { name: "Profile <test>" }),
    ).toBe("歡迎回來，Profile <test>");
    expect(translate("An untranslated future label")).toBe(
      "An untranslated future label",
    );
    expect(translate("{{count}} requests", { count: 2 })).toBe("2 個請求");
    await appI18n.changeLanguage("en");
    expect(translate("{{count}} requests", { count: 1 })).toBe("1 request");
    expect(translate("{{count}} requests", { count: 2 })).toBe("2 requests");
  });

  it("translates breadcrumb labels while preserving an owner-chosen name", async () => {
    await appI18n.changeLanguage("zh-CN");
    expect(
      buildStudioBreadcrumbs(
        "/keys/credential-1",
        { "/keys/credential-1": "Profile" },
        {},
        undefined,
        translate,
      ).map((crumb) => crumb.label),
    ).toEqual(["服务与凭据", "外部服务", "Profile", "概览"]);
  });

  it.each([
    ["en", "Apr 15", "1.25 credits"],
    ["zh-CN", "4月15日", "1.25 额度"],
    ["zh-TW", "4月15日", "1.25 額度"],
  ])("updates chart dates and billing units in %s", async (language, date, credits) => {
    await appI18n.changeLanguage(language);
    expect(formatBucketDate("2026-04-15")).toBe(date);
    expect(formatEstimatedCredits("1.25")).toBe(credits);
  });

  it("switches open tabs and updates the document language", () => {
    window.dispatchEvent(
      new StorageEvent("storage", {
        key: LANGUAGE_STORAGE_KEY,
        newValue: "zh-TW",
      }),
    );
    expect(appI18n.language).toBe("zh-TW");
    expect(document.documentElement.lang).toBe("zh-TW");
    expect(translate("Settings")).toBe("設定");
  });
});
