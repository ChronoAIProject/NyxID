/** A message instant, formatted for the reader's own locale and time zone. */
export interface MessageTimeLabels {
  /** The instant itself, for `<time dateTime>`. */
  readonly iso: string;
  /** Local date, time and time zone. */
  readonly label: string;
}

export interface MessageTimeOptions {
  /** Tests pin these; the browser's own apply otherwise. */
  readonly locale?: string;
  readonly timeZone?: string;
}

const LABEL = {
  year: "numeric",
  month: "short",
  day: "numeric",
  hour: "numeric",
  minute: "2-digit",
  timeZoneName: "short",
} as const;

const formatters = new Map<string, Intl.DateTimeFormat>();

function formatter({ locale, timeZone }: MessageTimeOptions): Intl.DateTimeFormat {
  const key = `${locale ?? ""}|${timeZone ?? ""}`;
  let cached = formatters.get(key);
  if (!cached) {
    cached = new Intl.DateTimeFormat(locale, { ...LABEL, timeZone });
    formatters.set(key, cached);
  }
  return cached;
}

/**
 * Display label for a stored message instant (epoch milliseconds or an ISO
 * string). It always names the date and zone, so messages from different days
 * or either side of a clock change never read alike. Missing or unparseable
 * instants yield nothing rather than `Invalid Date` or the current time.
 */
export function messageTime(
  value: number | string | null | undefined,
  options: MessageTimeOptions = {},
): MessageTimeLabels | undefined {
  if (value == null || value === "") return undefined;
  const date = new Date(value);
  if (!Number.isFinite(date.getTime())) return undefined;
  return { iso: date.toISOString(), label: formatter(options).format(date) };
}
