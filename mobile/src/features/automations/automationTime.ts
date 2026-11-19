import { appLocale } from "../../i18n/appTranslation";
/** Times always use the owner's selected IANA timezone. */
export const deviceTimezone = () =>
  Intl.DateTimeFormat(appLocale()).resolvedOptions().timeZone || "UTC";

export function formatAutomationTime(
  value: string | null | undefined,
  timezone: string,
) {
  return value
    ? new Intl.DateTimeFormat(appLocale(), {
        timeZone: timezone,
        dateStyle: "medium",
        timeStyle: "short",
      }).format(new Date(value))
    : "—";
}
