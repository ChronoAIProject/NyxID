import { createInstance } from "i18next";
import { initReactI18next } from "react-i18next";
import { config as configureZod } from "zod";
import zodEnglish from "zod/v4/locales/en.js";
import zodSimplifiedChinese from "zod/v4/locales/zh-CN.js";
import zodTraditionalChinese from "zod/v4/locales/zh-TW.js";
import { setFormattingLocale } from "@/lib/formatting-locale";
import {
  initialLanguage,
  LANGUAGE_STORAGE_KEY,
  resolveLanguage,
} from "./language";
import commonEn from "./locales/common-en.json";
import commonZhCN from "./locales/common-zh-CN.json";
import commonZhTW from "./locales/common-zh-TW.json";
import errorsEn from "./locales/errors-en.json";
import errorsZhCN from "./locales/errors-zh-CN.json";
import errorsZhTW from "./locales/errors-zh-TW.json";
import { catalogBackend } from "./catalog-loader";

// Shared by the app and standalone CLI wizard; each entry registers its own catalogs.
export const appI18n = createInstance();
void appI18n.use(catalogBackend).use(initReactI18next).init({
  resources: {
    en: { common: commonEn, errors: errorsEn },
    "zh-CN": { common: commonZhCN, errors: errorsZhCN },
    "zh-TW": { common: commonZhTW, errors: errorsZhTW },
  },
  lng: initialLanguage(),
  supportedLngs: ["en", "zh-CN", "zh-TW"],
  fallbackLng: "en",
  defaultNS: "translation",
  ns: ["common", "errors"],
  partialBundledLanguages: true,
  initAsync: false,
  interpolation: { escapeValue: false },
});

function applyLanguage(language: string) {
  configureZod(
    language === "zh-CN" ? zodSimplifiedChinese() :
    language === "zh-TW" ? zodTraditionalChinese() : zodEnglish(),
  );
  if (typeof document !== "undefined") document.documentElement.lang = language;
  setFormattingLocale(language, (message, count) =>
    appI18n.t(message, {
      ns: "common",
      lng: language,
      defaultValue: message,
      keySeparator: false,
      nsSeparator: false,
      ...(count === undefined ? {} : { count }),
    }),
  );
}
applyLanguage(appI18n.language);
appI18n.on("languageChanged", (language) => {
  applyLanguage(language);
  try {
    localStorage.setItem(LANGUAGE_STORAGE_KEY, language);
  } catch {
    // Language switching also works when persistence is unavailable.
  }
});
if (typeof window !== "undefined") {
  window.addEventListener("storage", (event) => {
    if (event.key !== LANGUAGE_STORAGE_KEY && event.key !== null) return;
    const language = resolveLanguage(event.newValue) ?? initialLanguage();
    if (appI18n.language !== language) void appI18n.changeLanguage(language);
  });
}
