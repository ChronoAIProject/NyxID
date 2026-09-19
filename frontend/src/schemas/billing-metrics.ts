import { z } from "zod";

export const BILLING_METRICS = [
  "tokens",
  "requests",
  "bytes",
  "input_tokens",
  "output_tokens",
  "cache_read_tokens",
  "cache_write_tokens",
  "images",
] as const;
export const BILLING_UNITS: Record<
  (typeof BILLING_METRICS)[number],
  { label: string; singular: string; tokenFamily: boolean }
> = {
  tokens: { label: "tokens", singular: "token", tokenFamily: true },
  requests: { label: "requests", singular: "request", tokenFamily: false },
  bytes: { label: "bytes", singular: "byte", tokenFamily: false },
  input_tokens: {
    label: "input tokens",
    singular: "input token",
    tokenFamily: true,
  },
  output_tokens: {
    label: "output tokens",
    singular: "output token",
    tokenFamily: true,
  },
  cache_read_tokens: {
    label: "cache-read tokens",
    singular: "cache-read token",
    tokenFamily: true,
  },
  cache_write_tokens: {
    label: "cache-write tokens",
    singular: "cache-write token",
    tokenFamily: true,
  },
  images: { label: "images", singular: "image", tokenFamily: false },
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
