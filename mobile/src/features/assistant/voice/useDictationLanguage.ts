import { useEffect, useState } from "react";
import { ExpoSpeechRecognitionModule as Recognizer } from "expo-speech-recognition";
import { useAppLanguage } from "../../../i18n/LanguageProvider";
import { dictationLocale } from "./dictationLocale";
import {
  languageLabel,
  languageOptions,
  saveDictationLanguage,
  savedDictationLanguage,
} from "./dictationLanguage";

/**
 * The dictation language: the person's choice when the recognizer supports
 * it, else the device's. Changing it applies from the next recording.
 */
export function useDictationLanguage(active: boolean) {
  const { t, language } = useAppLanguage();
  const [saved, setSaved] = useState<string>();
  const [supported, setSupported] = useState<readonly string[]>([]);
  useEffect(() => {
    if (!active) return;
    let live = true;
    void savedDictationLanguage().then((value) => {
      if (live) setSaved(value);
    });
    void Recognizer.getSupportedLocales({})
      .then((result) => {
        if (live) setSupported(result.locales);
      })
      .catch(() => {});
    return () => {
      live = false;
    };
  }, [active]);
  let phone: string | undefined;
  try {
    phone = Intl.DateTimeFormat().resolvedOptions().locale;
  } catch {
    phone = undefined;
  }
  const automatic = phone
    ? (dictationLocale(phone, supported) ?? phone)
    : undefined;
  const selected =
    saved && (!supported.length || supported.includes(saved))
      ? saved
      : undefined;
  const current = selected ?? automatic;
  return {
    value: selected ?? AUTOMATIC,
    label: current
      ? languageLabel(current, language)
      : t("Your device's language"),
    automaticLabel: automatic
      ? `${languageLabel(automatic, language)} · ${automatic}`
      : t("Your device's language"),
    options: [
      {
        value: AUTOMATIC,
        label: t("Device language"),
        detail: automatic
          ? `${languageLabel(automatic, language)} · ${automatic}`
          : undefined,
      },
      ...languageOptions(supported, language),
    ],
    choose: (value: string) => {
      const next = value === AUTOMATIC ? undefined : value;
      setSaved(next);
      void saveDictationLanguage(next);
    },
  };
}
const AUTOMATIC = "automatic";
