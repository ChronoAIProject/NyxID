const SCALE = 1_000_000_000_000n;
const MAX = 10n ** 34n - 1n;
const DECIMAL = /^-?\d+(?:\.\d{1,12})?$/;

/** API Credits are canonical decimal strings at a precision of 10^-12. */
export function creditUnits(value: string): bigint {
  if (value.length > 80 || !DECIMAL.test(value)) throw new Error("Invalid credit amount");
  const negative = value.startsWith("-");
  const [whole, fraction = ""] = (negative ? value.slice(1) : value).split(".");
  const units = BigInt(whole!) * SCALE + BigInt(fraction.padEnd(12, "0"));
  if (units > MAX) throw new Error("Credit amount exceeds supported precision");
  return negative ? -units : units;
}

export function decimalCredits(units: bigint): string {
  const absolute = units < 0n ? -units : units;
  const fraction = (absolute % SCALE).toString().padStart(12, "0").replace(/0+$/, "");
  return `${units < 0n ? "-" : ""}${absolute / SCALE}${fraction ? `.${fraction}` : ""}`;
}

export function formatCredits(value: string | null | undefined): string {
  if (value == null) return "Unavailable";
  const canonical = decimalCredits(creditUnits(value));
  const [whole, fraction] = canonical.split(".");
  return `${whole!.replace(/\B(?=(\d{3})+(?!\d))/g, ",")}${fraction ? `.${fraction}` : ""}`;
}

function creditLocale(locale?: string) {
  // Hermes exposes neither formatToParts nor BigInt formatting. Fixed integer
  // probes discover separators/group widths; money itself never enters Intl.
  const formatter = new Intl.NumberFormat(locale, { numberingSystem: "latn", maximumFractionDigits: 0 });
  const sample = formatter.format(1_234_567_890);
  const groups = sample.split(/[^0-9]+/).filter(Boolean);
  const separator = sample.match(/[^0-9]+/)?.[0] ?? "";
  const primary = groups.at(-1)!.length;
  const secondary = groups.at(-2)?.length ?? primary;
  const minimum = /[^0-9]/.test(formatter.format(1_000)) ? 4 : 5;
  const decimal = new Intl.NumberFormat(locale, { numberingSystem: "latn", useGrouping: false, minimumFractionDigits: 1, maximumFractionDigits: 1 })
    .format(0).replace(/[0-9]/g, "");
  return {
    decimal,
    group(whole: string) {
      if (!separator || whole.length < minimum) return whole;
      const pieces: string[] = [];
      let end = whole.length;
      let width = primary;
      while (end > 0) {
        pieces.unshift(whole.slice(Math.max(0, end - width), end));
        end -= width;
        width = secondary;
      }
      return pieces.join(separator);
    },
  };
}

/** Matches the web's six-decimal, half-away-from-zero credit display. */
export function formatExactCredits(value: string, locale?: string): string {
  const pico = creditUnits(value);
  const magnitude = pico < 0n ? -pico : pico;
  const step = 1_000_000n;
  const syntax = creditLocale(locale);
  if (magnitude > 0n && magnitude < step) return `${pico < 0n ? ">-" : "<"}0${syntax.decimal}000001`;
  const rounded = ((magnitude + step / 2n) / step) * step;
  const [whole, fraction] = decimalCredits(rounded).split(".");
  return `${pico < 0n ? "-" : ""}${syntax.group(whole!)}${fraction ? syntax.decimal + fraction : ""}`;
}

export function sumCredits(values: readonly (string | null)[]): string | null {
  if (values.some((value) => value === null)) return null;
  return decimalCredits(values.reduce<bigint>((sum, value) => sum + creditUnits(value!), 0n));
}

/** Compatibility integers are admitted only when JS can represent them exactly. */
export function legacyCredits(value: number, scale: "whole" | "micros"): string {
  if (!Number.isSafeInteger(value)) throw new Error("Unsafe legacy credit amount");
  return decimalCredits(BigInt(value) * (scale === "whole" ? SCALE : 1_000_000n));
}

export function expiryLabel(value: string | null | undefined, now = Date.now()): string {
  if (!value) return "No expiry";
  const date = new Date(value);
  if (!Number.isFinite(date.getTime())) throw new Error("Invalid expiry");
  return `${date.getTime() <= now ? "Expired" : "Expires"} ${date.toLocaleDateString(undefined, { year: "numeric", month: "short", day: "numeric", timeZone: "UTC" })} (UTC)`;
}

export function billingDate(value: string): string {
  return new Date(value).toLocaleDateString(undefined, { year: "numeric", month: "short", day: "numeric", timeZone: "UTC" });
}

export const formatQuantity = (value: number) => BigInt(value).toString().replace(/\B(?=(\d{3})+(?!\d))/g, ",");
