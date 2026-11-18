import type { Language } from "./language";
import { translate } from "./translate";

// Pure formatting helpers and event handlers share the provider's selected language.
// Components subscribe through useAppLanguage; no device locale is read here.
let selectedLanguage: Language = "en";
export function setAppTranslationLanguage(language: Language) {
  selectedLanguage = language;
}
export function appLocale() {
  return selectedLanguage;
}
export function translateApp(phrase: string, values?: Record<string, unknown>) {
  return translate(
    selectedLanguage,
    phrase,
    values
      ? Object.fromEntries(
          Object.entries(values).map(([key, value]) => [key, String(value)]),
        )
      : undefined,
  );
}
