import { createInstance } from "i18next";
import { initReactI18next } from "react-i18next";
import type { Language } from "./website-context";

export function initialLanguage(): Language {
  return typeof window !== "undefined" && (window.location.pathname === "/zh" || window.location.pathname.startsWith("/zh/")) ? "zh" : "en";
}

// A separate instance avoids overwriting AuthFlow/app translation resources.
export const websiteI18n = createInstance();
void websiteI18n.use(initReactI18next).init({
  lng: initialLanguage(),
  fallbackLng: "en",
  initAsync: false,
  resources: { en: { translation: {} }, zh: { translation: {} } },
});
