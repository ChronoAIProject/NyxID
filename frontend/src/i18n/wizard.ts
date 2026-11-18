import { useCallback } from "react";
import type { TOptions } from "i18next";
import { translateErrorMessage } from "@/lib/error-messages";
import { useTranslation } from "react-i18next";
import { appI18n } from "./instance";
import en from "./locales/wizard-en.json";
import zhCN from "./locales/wizard-zh-CN.json";
import zhTW from "./locales/wizard-zh-TW.json";

for (const [language, resources] of Object.entries({
  en,
  "zh-CN": zhCN,
  "zh-TW": zhTW,
})) {
  appI18n.addResourceBundle(language, "wizard", resources);
}

export function translateWizard(key: string, options?: TOptions): string {
  const errorCopy = translateErrorMessage(key, options);
  if (errorCopy !== key) return errorCopy;
  return appI18n.t(key, {
    ...options,
    ns: ["wizard", "errors"],
    keySeparator: false,
    nsSeparator: false,
    defaultValue: key,
    nest: false,
  });
}

export function useWizardTranslation() {
  const { i18n } = useTranslation("wizard", { i18n: appI18n });
  const language = i18n.resolvedLanguage;
  const t = useCallback(
    (key: string, options?: TOptions) =>
      translateWizard(key, { ...options, lng: language }),
    [language],
  );
  return { t, i18n };
}
