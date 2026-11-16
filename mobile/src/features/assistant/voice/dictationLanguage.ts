import AsyncStorage from "@react-native-async-storage/async-storage";
import localeNames from "../../../i18n/dictationLocaleNames.json";
import type { Language } from "../../../i18n/language";

/** The person's dictation language; absent means "follow the device". */
export const DICTATION_LANGUAGE_KEY = "nyxid.dictation_language";

export async function savedDictationLanguage(): Promise<string | undefined> {
  try {
    return (await AsyncStorage.getItem(DICTATION_LANGUAGE_KEY)) ?? undefined;
  } catch {
    return undefined;
  }
}

export async function saveDictationLanguage(locale: string | undefined) {
  try {
    if (locale) await AsyncStorage.setItem(DICTATION_LANGUAGE_KEY, locale);
    else await AsyncStorage.removeItem(DICTATION_LANGUAGE_KEY);
  } catch {
    // A failed write keeps the previous choice; dictation still works.
  }
}

// CLDR display names bundled for Hermes: English, simplified Chinese,
// traditional Chinese, then the language's own name (languages only).
const languages: Record<string, readonly string[]> = localeNames.languages;
const regions: Record<string, readonly string[]> = localeNames.regions;
const languageIndex = { en: 0, "zh-CN": 1, "zh-TW": 2 } as const;

function localeParts(locale: string) {
  const [base = "", ...parts] = locale.replace(/_/g, "-").split("-");
  const script = parts.find((part) => /^[a-z]{4}$/i.test(part));
  const region = parts.find((part) => /^([a-z]{2}|\d{3})$/i.test(part));
  const language = base.toLowerCase();
  return {
    language,
    script:
      script &&
      `${language}-${script[0]!.toUpperCase()}${script.slice(1).toLowerCase()}`,
    region: region?.toUpperCase(),
  };
}

/** Names stay readable on Hermes, where Intl.DisplayNames is unavailable. */
export function languageLabel(
  locale: string,
  displayLanguage: Language = "en",
): string {
  const tag = localeParts(locale);
  const name = languages[tag.script ?? ""] ?? languages[tag.language];
  const label = name?.[languageIndex[displayLanguage]];
  if (label) {
    const region = tag.region
      ? (regions[tag.region]?.[languageIndex[displayLanguage]] ?? tag.region)
      : undefined;
    return region ? `${label} (${region})` : label;
  }
  try {
    const names = new Intl.DisplayNames(displayLanguage, { type: "language" });
    return names.of(locale.replace(/_/g, "-")) ?? locale;
  } catch {
    return locale;
  }
}

/** Supported recognizer locales as menu options, sorted by their names. */
export function languageOptions(
  supported: readonly string[],
  displayLanguage: Language = "en",
) {
  return [...new Set(supported)]
    .map((value) => {
      const tag = localeParts(value);
      const nativeName = (languages[tag.script ?? ""] ??
        languages[tag.language])?.[3];
      return {
        value,
        label: languageLabel(value, displayLanguage),
        detail: nativeName ? `${nativeName} · ${value}` : value,
      };
    })
    .sort((a, b) => a.label.localeCompare(b.label));
}
