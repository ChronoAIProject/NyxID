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

export {
  languageLabel,
  languageOptions,
  matchesLanguage,
} from "./dictationLanguageLabels";

export const RECENT_DICTATION_LANGUAGES_KEY = "nyxid.dictation_language_recent";
export async function recentDictationLanguages(): Promise<string[]> {
  try {
    const value: unknown = JSON.parse(
      (await AsyncStorage.getItem(RECENT_DICTATION_LANGUAGES_KEY)) ?? "[]",
    );
    return Array.isArray(value)
      ? value.filter((x): x is string => typeof x === "string").slice(0, 3)
      : [];
  } catch {
    return [];
  }
}
export async function rememberDictationLanguage(locale: string) {
  const recent = await recentDictationLanguages();
  try {
    await AsyncStorage.setItem(
      RECENT_DICTATION_LANGUAGES_KEY,
      JSON.stringify(
        [locale, ...recent.filter((value) => value !== locale)].slice(0, 3),
      ),
    );
  } catch {
    // Selection remains usable for this recording.
  }
}
