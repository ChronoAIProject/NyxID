import { expect, it } from "vitest";
import { appI18n, appLanguageReady, translate } from ".";

it("loads each Chinese app catalog when selected and retains the English fallback", async () => {
  await appLanguageReady;
  expect(appI18n.hasResourceBundle("en", "app")).toBe(true);
  expect(appI18n.hasResourceBundle("zh-CN", "app")).toBe(false);
  expect(appI18n.hasResourceBundle("zh-TW", "app")).toBe(false);

  await appI18n.changeLanguage("zh-CN");
  expect(translate("Settings")).toBe("设置");
  expect(appI18n.hasResourceBundle("zh-CN", "app")).toBe(true);
  expect(appI18n.hasResourceBundle("zh-TW", "app")).toBe(false);

  await appI18n.changeLanguage("zh-TW");
  expect(translate("Settings")).toBe("設定");
  expect(appI18n.hasResourceBundle("zh-TW", "app")).toBe(true);
  await appI18n.changeLanguage("en");
  expect(translate("Settings")).toBe("Settings");
});
