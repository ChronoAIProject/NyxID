import AsyncStorage from "@react-native-async-storage/async-storage";

/** The person's dictation language; absent means "follow the iPhone". */
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

/** "English (United States)", in the reader's own language where possible. */
export function languageLabel(locale: string): string {
  try {
    const names = new Intl.DisplayNames(undefined, { type: "language" });
    return names.of(locale.replace(/_/g, "-")) ?? locale;
  } catch {
    return locale;
  }
}

/** Supported recognizer locales as menu options, sorted by their names. */
export function languageOptions(supported: readonly string[]) {
  return [...new Set(supported)]
    .map((value) => ({ value, label: languageLabel(value) }))
    .sort((a, b) => a.label.localeCompare(b.label));
}
