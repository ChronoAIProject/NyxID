import { appI18n } from "@/i18n/instance";

/** Active UI locale, used only as a hint when the first message's language is unclear. */
export function assistantTitleLanguage(): string {
  return appI18n.resolvedLanguage ?? appI18n.language;
}
