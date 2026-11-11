type TimeTranslator = (message: string, count?: number) => string;

let locale = "en-US";
let timeTranslator: TimeTranslator = (message, count) =>
  count === undefined
    ? message
    : message.replaceAll("{{count}}", String(count));

/** The web app supplies its translator; standalone bundles keep English defaults. */
export function setFormattingLocale(
  language: string,
  translateTime: TimeTranslator,
) {
  locale = language === "en" ? "en-US" : language;
  timeTranslator = translateTime;
}

export function getFormattingLocale(): string {
  return locale;
}

const calendarDateFormatters = new Map<string, Intl.DateTimeFormat>();
export function getCalendarDateFormatter(): Intl.DateTimeFormat {
  let formatter = calendarDateFormatters.get(locale);
  if (!formatter) {
    formatter = new Intl.DateTimeFormat(locale, {
      month: "short",
      day: "numeric",
      year: "numeric",
      timeZone: "UTC",
    });
    calendarDateFormatters.set(locale, formatter);
  }
  return formatter;
}

export function translateTime(
  message: string,
  options?: { readonly count: number },
): string {
  return timeTranslator(message, options?.count);
}
