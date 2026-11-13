import type { TOptions } from "i18next";
import { translateErrorMessage } from "@/lib/error-messages";
import { appI18n } from "./instance";
export { appI18n } from "./instance";
import landingEn from "@/features/landing/i18n/locales/en.json";
import landingZhCN from "@/features/landing/i18n/locales/zh-CN.json";
import landingZhTW from "@/features/landing/i18n/locales/zh-TW.json";
import onboardingEn from "@/features/nyxbot-onboarding/locales/en.json";
import onboardingZhCN from "@/features/nyxbot-onboarding/locales/zh-CN.json";
import onboardingZhTW from "@/features/nyxbot-onboarding/locales/zh-TW.json";
import en from "./locales/en-overrides.json";
import { registerCatalogLoader, waitForCatalogs } from "./catalog-loader";

appI18n.addResourceBundle("en", "app", en);
registerCatalogLoader("app", async (language) => {
  if (language === "zh-CN") return (await import("./locales/zh-CN.json")).default;
  if (language === "zh-TW") return (await import("./locales/zh-TW.json")).default;
  return en;
});

for (const [language, namespaces] of Object.entries({
  en: { translation: landingEn, onboarding: onboardingEn },
  "zh-CN": { translation: landingZhCN, onboarding: onboardingZhCN },
  "zh-TW": { translation: landingZhTW, onboarding: onboardingZhTW },
})) {
  for (const [namespace, resources] of Object.entries(namespaces)) {
    appI18n.addResourceBundle(language, namespace, resources);
  }
}

export const appLanguageReady = waitForCatalogs(appI18n.loadNamespaces("app"));

/** Translate app-owned copy only; server data and user content stay verbatim. */
export function translate(key: string, options?: TOptions): string {
  const errorCopy = translateErrorMessage(key, options);
  if (errorCopy !== key) return errorCopy;
  return appI18n.t(key, {
    ...options,
    ns: ["app", "errors"],
    keySeparator: false,
    nsSeparator: false,
    defaultValue: key,
    nest: false,
  });
}
