import { translateUi } from "@/i18n/use-ui-translation";
import { z } from "zod";

export const SERVICE_BILLING_METRICS = [
  "tokens",
  "requests",
  "bytes",
  "input_tokens",
  "output_tokens",
  "cache_read_tokens",
  "cache_write_tokens",
  "images",
  "voice_seconds",
] as const;
// Runtime billing has its own logical SKU; ordinary service lanes cannot price it.
export const BILLING_METRICS = [
  ...SERVICE_BILLING_METRICS,
  "machine_running_seconds",
] as const;
export const BILLING_UNITS: Record<
  (typeof BILLING_METRICS)[number],
  { label: string; singular: string; tokenFamily: boolean }
> = {
  tokens: { get label() { return translateUi("tokens"); }, get singular() { return translateUi("token"); }, tokenFamily: true },
  requests: { get label() { return translateUi("requests"); }, get singular() { return translateUi("request"); }, tokenFamily: false },
  bytes: { get label() { return translateUi("bytes"); }, get singular() { return translateUi("byte"); }, tokenFamily: false },
  input_tokens: {
    get label() { return translateUi("input tokens"); },
    get singular() { return translateUi("input token"); },
    tokenFamily: true,
  },
  output_tokens: {
    get label() { return translateUi("output tokens"); },
    get singular() { return translateUi("output token"); },
    tokenFamily: true,
  },
  cache_read_tokens: {
    get label() { return translateUi("cache-read tokens"); },
    get singular() { return translateUi("cache-read token"); },
    tokenFamily: true,
  },
  cache_write_tokens: {
    get label() { return translateUi("cache-write tokens"); },
    get singular() { return translateUi("cache-write token"); },
    tokenFamily: true,
  },
  voice_seconds: {
    get label() { return translateUi("voice seconds"); },
    get singular() { return translateUi("voice second"); },
    tokenFamily: false,
  },
  machine_running_seconds: {
    get label() { return translateUi("machine running seconds"); },
    get singular() { return translateUi("machine running second"); },
    tokenFamily: false,
  },
  images: { get label() { return translateUi("images"); }, get singular() { return translateUi("image"); }, tokenFamily: false },
};
export function metricLabel(metric: string, quantity?: number): string {
  const unit = BILLING_UNITS[metric as keyof typeof BILLING_UNITS];
  return unit ? (quantity === 1 ? unit.singular : unit.label) : metric;
}
export const PRICE_FRACTIONAL_DIGITS = 12;
export const PRICE_PATTERN = new RegExp(
  `^\\d+(?:\\.\\d{1,${PRICE_FRACTIONAL_DIGITS}})?$`,
);
export function validUnitPrice(value: string): boolean {
  if (!PRICE_PATTERN.test(value)) return false;
  const [whole, fraction = ""] = value.split(".");
  return (
    BigInt(whole!) * 10n ** 12n + BigInt(fraction.padEnd(12, "0")) <=
    1_000_000n * 10n ** 12n
  );
}
export const unitPriceSchema = z
  .string()
  .trim()
  .regex(
    PRICE_PATTERN,
    "Use a non-negative decimal with at most 12 decimal places",
  )
  .refine(validUnitPrice, "Price must not exceed 1,000,000 credits per unit");
