import { useCallback } from "react";
import type { TOptions } from "i18next";
import { useTranslation } from "react-i18next";
import { appI18n } from "./instance";
import { translateErrorMessage } from "@/lib/error-messages";

// Shared controls use the catalogs registered by their entry point. Importing
// this hook never pulls the Studio catalogs into the standalone CLI wizard.
export function translateUi(key: string, options?: TOptions): string {
  const errorCopy = translateErrorMessage(key, options);
  if (errorCopy !== key) return errorCopy;
  return appI18n.t(key, {
    ...options,
    ns: ["app", "wizard", "errors", "common"],
    keySeparator: false,
    nsSeparator: false,
    defaultValue: key,
    nest: false,
  });
}

export function useUiTranslation() {
  const { i18n } = useTranslation("common", {
    i18n: appI18n,
    useSuspense: false,
    bindI18n: "languageChanged loaded",
  });
  const language = i18n.resolvedLanguage;
  const t = useCallback(
    (key: string, options?: TOptions) =>
      translateUi(key, { ...options, lng: language }),
    [language],
  );
  return { t, i18n };
}
