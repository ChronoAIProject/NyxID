/** Calendar input and display always use the owner's selected IANA timezone. */
export const browserTimezone = () =>
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

export function localTime(value: string, timezone: string): string {
  if (!value) return "";
  if (!/[zZ]|[+-]\d\d:\d\d$/.test(value)) return value;
  const parts = new Intl.DateTimeFormat("en-CA", {
    timeZone: timezone,
    year: "numeric",
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
    hourCycle: "h23",
  }).formatToParts(new Date(value));
  const field = (name: Intl.DateTimeFormatPartTypes) =>
    parts.find((p) => p.type === name)!.value;
  return `${field("year")}-${field("month")}-${field("day")}T${field("hour")}:${field("minute")}`;
}

/** Earliest instant in a fold; first valid instant after a gap, matching cron. */
export function automationInstant(value: string, timezone: string): string {
  if (/[zZ]|[+-]\d\d:\d\d$/.test(value)) return new Date(value).toISOString();
  if (!/^\d{4}-\d\d-\d\dT\d\d:\d\d$/.test(value))
    throw new Error("Choose a date and time");
  const nominal = Date.parse(`${value}:00Z`);
  if (
    !Number.isFinite(nominal) ||
    new Date(nominal).toISOString().slice(0, 16) !== value
  )
    throw new Error("Choose a valid date and time");
  const wall = (utc: number) =>
    Date.parse(`${localTime(new Date(utc).toISOString(), timezone)}:00Z`);
  const offsets = new Set(
    [-36, 0, 36].map((hours) => {
      const at = nominal + hours * 3600_000;
      return wall(at) - at;
    }),
  );
  const candidates = [...offsets]
    .map((offset) => nominal - offset)
    .sort((a, b) => a - b);
  const matches = candidates.filter((at) => wall(at) === nominal);
  if (matches.length) return new Date(matches[0]!).toISOString();
  // A missing local time lies between the two UTC candidates on either side
  // of the transition. Locate the first valid minute rather than shifting it.
  let low = candidates[0]!;
  let high = candidates.at(-1)!;
  if (wall(low) >= nominal || wall(high) < nominal)
    throw new Error("Unable to resolve timezone transition");
  while (high - low > 60_000) {
    const mid = Math.floor((low + high) / 120_000) * 60_000;
    if (wall(mid) >= nominal) high = mid;
    else low = mid;
  }
  return new Date(high).toISOString();
}
