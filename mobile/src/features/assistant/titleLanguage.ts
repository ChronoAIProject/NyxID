import { appLocale } from "../../i18n/appTranslation";
/** Native follows the device's language; supplied explicitly to title generation. */
export function assistantTitleLanguage(): string {
  try { return Intl.DateTimeFormat(appLocale()).resolvedOptions().locale || "en"; } catch { return "en"; }
}
