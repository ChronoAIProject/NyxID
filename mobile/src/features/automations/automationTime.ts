/** Times always use the owner's selected IANA timezone. */
export const deviceTimezone = () =>
  Intl.DateTimeFormat().resolvedOptions().timeZone || "UTC";

export function formatAutomationTime(
  value: string | null | undefined,
  timezone: string,
) {
  return value
    ? new Intl.DateTimeFormat(undefined, {
        timeZone: timezone,
        dateStyle: "medium",
        timeStyle: "short",
      }).format(new Date(value))
    : "—";
}
