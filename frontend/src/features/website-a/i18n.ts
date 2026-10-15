import { createInstance } from "i18next";
import { initReactI18next } from "react-i18next";
import type { Language } from "./website-context";

export function initialLanguage(): Language {
  const requested = new URLSearchParams(window.location.search).get("lang");
  if (requested === "en" || requested === "zh") return requested;
  try {
    const saved = localStorage.getItem("nyx-lang");
    if (saved === "en" || saved === "zh") return saved;
  } catch {
    // The browser language is still available when persistent storage is blocked.
  }
  return navigator.language.toLowerCase().startsWith("zh") ? "zh" : "en";
}

// A separate instance avoids overwriting AuthFlow/app translation resources.
export const websiteI18n = createInstance();
void websiteI18n.use(initReactI18next).init({
  lng: initialLanguage(),
  fallbackLng: "en",
  initAsync: false,
  resources: { en: { translation: {} }, zh: { translation: {} } },
});
