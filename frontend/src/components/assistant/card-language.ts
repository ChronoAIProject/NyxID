import { createContext, useCallback, useContext, useEffect, useState } from "react";
import { subscribeAssistantIdentity } from "@/lib/assistant/identity";
import { resolveLanguage, type Language } from "@/i18n/language";
import { useUiTranslation, translateUi } from "@/i18n/use-ui-translation";
import type { CardLanguageChoice } from "@/lib/assistant/card-language";

export const CardLanguageContext = createContext<Language | undefined>(undefined);

export function useCardTranslation() {
  const { i18n } = useUiTranslation();
  const language = useContext(CardLanguageContext) ?? resolveLanguage(i18n.language) ?? "en";
  const t = useCallback<typeof translateUi>((key, options) =>
    translateUi(key, { ...options, lng: language }), [language]);
  return { language, t };
}

const storagePrefix = "nyxid.card-language.";

function clearCardLanguageChoices() {
  try {
    for (const key of Object.keys(sessionStorage)) {
      if (key.startsWith(storagePrefix)) sessionStorage.removeItem(key);
    }
  } catch { /* Storage is optional. */ }
}

subscribeAssistantIdentity((_next, previous) => {
  if (previous) clearCardLanguageChoices();
});

export function validCardLanguageChoice(value: string | null): CardLanguageChoice {
  return value === "app" || value === "en" || value === "zh-CN" || value === "zh-TW" ? value : "auto";
}

export function useCardLanguageChoice(owner?: string, conversation?: string) {
  const key = owner && conversation ? storagePrefix + JSON.stringify([owner, conversation]) : undefined;
  function read(): CardLanguageChoice {
    try { return key ? validCardLanguageChoice(sessionStorage.getItem(key)) : "auto"; }
    catch { return "auto"; }
  }
  const [saved, setSaved] = useState(() => ({ key, choice: read() }));
  if (saved.key !== key) setSaved({ key, choice: read() });
  return {
    choice: saved.key === key ? saved.choice : read(),
    onChange(choice: CardLanguageChoice) {
      if (!key) return;
      setSaved({ key, choice });
      try {
        if (choice === "auto") sessionStorage.removeItem(key);
        else sessionStorage.setItem(key, choice);
      } catch { /* Keep the active choice when storage is unavailable. */ }
    },
  };
}

export function useCardCatalog(language: Language) {
  const { i18n } = useUiTranslation();
  useEffect(() => {
    if (!i18n.hasResourceBundle(language, "app")) void i18n.loadLanguages(language);
  }, [i18n, language]);
}
