/** A message instant, formatted for the reader's own locale and time zone. */
export interface MessageTimeLabels {
  /** The instant itself, for `<time dateTime>`. */
  readonly iso: string;
  /** Local date, time and time zone. */
  readonly label: string;
  /** Local day and month; the year too when it is not the current one. */
  readonly short: string;
}

export interface MessageTimeOptions {
  /** Overrides the browser's preferred language or local time zone. */
  readonly locale?: string;
  readonly timeZone?: string;
  /** The reader's present, which decides whether `short` names the year. */
  readonly now?: number | Date;
}

const LABEL = {
  year: "numeric",
  month: "short",
  day: "numeric",
  hour: "numeric",
  minute: "2-digit",
  timeZoneName: "short",
} as const;
const SHORT = { day: "numeric", month: "short" } as const;
const SHORT_WITH_YEAR = { ...SHORT, year: "numeric" } as const;
const YEAR = { year: "numeric" } as const;

const formatters = new Map<string, Intl.DateTimeFormat>();

function formatter(
  name: string,
  fields: Intl.DateTimeFormatOptions,
  locale: string | undefined,
  timeZone: string | undefined,
): Intl.DateTimeFormat {
  const key = `${name}|${locale ?? ""}|${timeZone ?? ""}`;
  let cached = formatters.get(key);
  if (!cached) {
    cached = new Intl.DateTimeFormat(locale, { ...fields, timeZone });
    formatters.set(key, cached);
  }
  return cached;
}

function readerLocale(locale: string | undefined): string | undefined {
  if (locale) return locale;
  const preferences = (
    globalThis as {
      navigator?: { languages?: readonly string[]; language?: string };
    }
  ).navigator;
  // Native has no browser language preferences; Intl uses the device locale.
  return (
    preferences?.languages?.[0] ??
    preferences?.language ??
    Intl.DateTimeFormat().resolvedOptions().locale
  );
}

/**
 * Display labels for a stored message instant (epoch milliseconds or an ISO
 * string). `label` always names the date and zone, so messages from different
 * days or either side of a clock change never read alike. `short` puts the
 * day before the month (`27 Oct`), which English readers get regardless of
 * their region. Missing or unparseable instants yield nothing rather than
 * `Invalid Date` or the current time.
 */
export function messageTime(
  value: number | string | null | undefined,
  { locale, timeZone, now = Date.now() }: MessageTimeOptions = {},
): MessageTimeLabels | undefined {
  if (value == null || value === "") return undefined;
  const date = new Date(value);
  if (!Number.isFinite(date.getTime())) return undefined;
  const reader = readerLocale(locale);
  const dayFirst = reader?.toLowerCase().startsWith("en") ? "en-GB" : reader;
  const year = formatter("year", YEAR, "en", timeZone);
  const thisYear = year.format(date) === year.format(now);
  return {
    iso: date.toISOString(),
    label: formatter("label", LABEL, reader, timeZone).format(date),
    short: thisYear
      ? formatter("short", SHORT, dayFirst, timeZone).format(date)
      : formatter("short-year", SHORT_WITH_YEAR, dayFirst, timeZone).format(
          date,
        ),
  };
}
