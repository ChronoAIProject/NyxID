import { useEffect, useState } from "react";
import { Linking, Platform } from "react-native";
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

import { recognitionLocales } from "./recognitionLocales";

const listeners = new Set<() => void>();
export const AUTOMATIC_DICTATION_LANGUAGE = "automatic";
/** The saved choice wins while supported; otherwise use device preference order. */
export function useDictationLanguage(active: boolean) {
  const { t, language } = useAppLanguage();
  const [refresh, setRefresh] = useState(0);
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
    setSupported(undefined);
    void recognitionLocales(Recognizer, Platform.OS)
      .then((result) => {
        if (live) setSupported(result?.locales ?? []);
      })
      .catch(() => {
        if (live) setSupported([]);
      });
    return () => {
      live = false;
      listeners.delete(read);
    };
  }, [active, refresh]);
  const loading = supported === undefined;
  const locales = supported ?? [];
  const listUnavailable = !loading && !locales.length;
  const devices = deviceLanguages();
  const preferred = devices
    .map((value) =>
      resolveDictationLanguage(undefined, [value], locales, Platform.OS),
    )
    .filter((value): value is string => !!value);
  const automatic = loading
    ? undefined
    : resolveDictationLanguage(undefined, devices, locales, Platform.OS);
  const resolved = loading
    ? saved
    : resolveDictationLanguage(saved, devices, locales, Platform.OS);
  const savedSelected =
    !!saved &&
    (loading ||
      !locales.length ||
      Platform.OS === "android" ||
      !!resolveDictationLanguage(saved, [], locales, "android"));
  const label =
    (resolved ?? (savedSelected ? saved : undefined))
      ? languageLabel((resolved ?? saved)!, language)
      : t("Your device's language");
  return {
    loading,
    listUnavailable,
    explanation: listUnavailable
      ? t(
          Platform.OS === "android"
            ? "Your speech service did not provide a language list. Retry, check the device’s speech service, or enter a language code to try (for example zh-CN, en-US or ja-JP). Support depends on the service."
            : "The language list is unavailable. Dictation uses your saved language or your device’s speech recognition default.",
        )
      : undefined,
    value: savedSelected ? saved! : AUTOMATIC_DICTATION_LANGUAGE,
    resolved,
    label,
    summary: savedSelected
      ? label
      : t("Device language · {{label}}", { label }),
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
            : t("Dictation uses your device’s speech recognition default."),
      },
      ...languageOptions(locales, language),
    ],
    retry: () => setRefresh((value) => value + 1),
    manualEntry: Platform.OS === "android" && listUnavailable,
    openSpeechSettings: () => {
      void Linking.sendIntent("android.settings.VOICE_INPUT_SETTINGS")
        .catch(() => Linking.openSettings())
        .catch(() => {});
    },
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
