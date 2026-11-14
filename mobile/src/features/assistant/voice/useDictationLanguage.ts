import { useEffect, useState } from "react";
import { ExpoSpeechRecognitionModule as Recognizer } from "expo-speech-recognition";
import { useAppLanguage } from "../../../i18n/LanguageProvider";
import { deviceLanguages } from "../../../i18n/deviceLanguages";
import { resolveDictationLanguage } from "./resolveDictationLanguage";
import {
  languageLabel,
  languageOptions,
  saveDictationLanguage,
  savedDictationLanguage,
  recentDictationLanguages,
  rememberDictationLanguage,
} from "./dictationLanguage";

const listeners = new Set<() => void>();
export const AUTOMATIC_DICTATION_LANGUAGE = "automatic";
/** The saved choice wins while supported; otherwise use device preference order. */
export function useDictationLanguage(active: boolean) {
  const { t, language } = useAppLanguage();
  const [saved, setSaved] = useState<string>();
  const [recent, setRecent] = useState<string[]>([]);
  const [supported, setSupported] = useState<readonly string[]>();
  useEffect(() => {
    if (!active) return;
    let live = true;
    const read = () => {
      void savedDictationLanguage().then((value) => {
        if (live) setSaved(value);
      });
      void recentDictationLanguages().then((value) => {
        if (live) setRecent(value);
      });
    };
    read();
    listeners.add(read);
    void Recognizer.getSupportedLocales({})
      .then((result) => {
        if (live) setSupported(result.locales);
      })
      .catch(() => {});
    return () => {
      live = false;
      listeners.delete(read);
    };
  }, [active]);
  const loading = supported === undefined;
  const locales = supported ?? [];
  const devices = deviceLanguages();
  const preferred = devices
    .map((value) => resolveDictationLanguage(undefined, [value], locales))
    .filter((value): value is string => !!value);
  const automatic = resolveDictationLanguage(undefined, devices, locales);
  const resolved = loading
    ? saved
    : resolveDictationLanguage(saved, devices, locales);
  const savedSelected = !!saved && (loading || locales.includes(saved));
  const label = resolved
    ? languageLabel(resolved, language)
    : t("Your device's language");
  return {
    loading,
    value: savedSelected ? saved! : AUTOMATIC_DICTATION_LANGUAGE,
    resolved,
    label,
    summary: savedSelected ? label : `${t("Device language")} · ${label}`,
    suggested: [
      ...new Set([
        ...preferred,
        ...recent.filter((value) => locales.includes(value)),
      ]),
    ],
    options: [
      {
        value: AUTOMATIC_DICTATION_LANGUAGE,
        label: t("Device language"),
        detail: automatic
          ? languageLabel(automatic, language)
          : loading
            ? undefined
            : t("No supported device language"),
      },
      ...languageOptions(locales, language),
    ],
    choose: (value: string) => {
      const next = value === AUTOMATIC_DICTATION_LANGUAGE ? undefined : value;
      setSaved(next);
      void (async () => {
        await saveDictationLanguage(next);
        if (next) await rememberDictationLanguage(next);
        listeners.forEach((listener) => listener());
      })();
    },
  };
}
