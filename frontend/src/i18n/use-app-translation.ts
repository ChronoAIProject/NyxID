import { useCallback } from "react";
import type { TOptions } from "i18next";
import { useTranslation } from "react-i18next";
import { appI18n, translate } from ".";

export function useAppTranslation() {
  const { i18n } = useTranslation("app", { i18n: appI18n });
  const language = i18n.resolvedLanguage;
  const t = useCallback(
    (key: string, options?: TOptions) =>
      translate(key, { ...options, lng: language }),
    [language],
  );
  return { t, i18n };
}
