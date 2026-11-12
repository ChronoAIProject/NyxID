import simplified from "./locales/zh-CN.json";
import traditional from "./locales/zh-TW.json";
import type { Language } from "./language";

export type TranslationValues = Record<string, string | number>;
const catalogs: Record<string, Record<string, string>> = {
  "zh-CN": simplified,
  "zh-TW": traditional,
};

/** Only explicitly marked app copy is translated; interpolated user data stays verbatim. */
export function translate(
  language: Language,
  phrase: string,
  values?: TranslationValues,
): string {
  const translated = catalogs[language]?.[phrase] ?? phrase;
  return translated.replace(/\{\{(\w+)\}\}/g, (placeholder, name: string) =>
    values && Object.hasOwn(values, name) ? String(values[name]) : placeholder,
  );
}
