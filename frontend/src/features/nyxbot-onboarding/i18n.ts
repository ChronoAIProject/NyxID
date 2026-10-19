import { createInstance } from "i18next";
import { initReactI18next } from "react-i18next";
import en from "./locales/en.json";
import zhCN from "./locales/zh-CN.json";

function initialLanguage() {
  try {
    const saved = localStorage.getItem("nyxbot-onboarding-language");
    if (saved === "en" || saved === "zh-CN") return saved;
  } catch {
    /* Storage may be unavailable in an embedded browser. */
  }
  return navigator.language.toLowerCase().startsWith("zh") ? "zh-CN" : "en";
}

// Keep the feature's resources separate from the landing page's default namespace.
export const nyxbotI18n = createInstance();
void nyxbotI18n.use(initReactI18next).init({
  resources: { en: { translation: en }, "zh-CN": { translation: zhCN } },
  lng: initialLanguage(),
  fallbackLng: "en",
  initAsync: false,
  interpolation: { escapeValue: false },
});
nyxbotI18n.on("languageChanged", (language) => {
  try {
    localStorage.setItem("nyxbot-onboarding-language", language);
  } catch {
    /* The in-memory language still works when storage is disabled. */
  }
});
