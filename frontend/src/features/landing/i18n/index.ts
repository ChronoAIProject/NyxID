import i18n from "i18next";
import { initReactI18next } from "react-i18next";

import en from "./locales/en.json";
import zhCN from "./locales/zh-CN.json";
import zhTW from "./locales/zh-TW.json";

function detectLangFromBrowser(): string {
  const lang = navigator.language?.toLowerCase() ?? "";
  // zh-TW, zh-Hant, zh-HK → Traditional Chinese
  if (
    lang.startsWith("zh-tw") ||
    lang.startsWith("zh-hant") ||
    lang.startsWith("zh-hk")
  )
    return "zh-TW";
  // zh, zh-CN, zh-Hans → Simplified Chinese
  if (lang.startsWith("zh")) return "zh-CN";
  return "en";
}

i18n.use(initReactI18next).init({
  resources: {
    en: { translation: en },
    "zh-CN": { translation: zhCN },
    "zh-TW": { translation: zhTW },
  },
  lng: detectLangFromBrowser(),
  fallbackLng: "en",
  interpolation: { escapeValue: false },
});

export default i18n;
