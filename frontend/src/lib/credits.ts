/** Decimal credits are transported as strings and calculated in picocredits. */
export const CREDIT_SCALE = 1_000_000_000_000n;
export function parseCredits(value: string): bigint {
  if (!/^-?\d+(?:\.\d{1,12})?$/.test(value)) throw new Error("Invalid credits");
  const negative = value.startsWith("-");
  const [whole, fraction = ""] = (negative ? value.slice(1) : value).split(".");
  const pico = BigInt(whole!) * CREDIT_SCALE + BigInt(fraction.padEnd(12, "0"));
  return negative ? -pico : pico;
}
export function decimalCredits(pico: bigint): string {
  const sign = pico < 0n ? "-" : "";
  const magnitude = pico < 0n ? -pico : pico;
  const fraction = (magnitude % CREDIT_SCALE)
    .toString()
    .padStart(12, "0")
    .replace(/0+$/, "");
  return `${sign}${magnitude / CREDIT_SCALE}${fraction ? `.${fraction}` : ""}`;
}
export function legacyCredits(
  value: number,
  unit: "whole" | "micros" = "micros",
): string {
  if (!Number.isSafeInteger(value))
    throw new Error("Legacy credits exceed integer precision");
  return decimalCredits(
    BigInt(value) * (unit === "whole" ? CREDIT_SCALE : 1_000_000n),
  );
}
export function exactCredits(
  exact: string | null | undefined,
  legacy: number | null | undefined,
  unit: "whole" | "micros" = "micros",
): string | null {
  return exact !== undefined
    ? exact
    : legacy == null
      ? null
      : legacyCredits(legacy, unit);
}
/** Six display decimals, half away from zero; tiny nonzero values stay visible. */
export function formatExactCredits(value: string, digits = 6): string {
  const pico = parseCredits(value);
  const magnitude = pico < 0n ? -pico : pico;
  const step = 10n ** BigInt(12 - digits);
  if (magnitude > 0n && magnitude < step)
    return `${pico < 0n ? ">-" : "<"}${decimalCredits(step)}`;
  const rounded = ((magnitude + step / 2n) / step) * step;
  const [whole, fraction] = decimalCredits(rounded).split(".");
  const grouped = new Intl.NumberFormat(undefined, {
    maximumFractionDigits: 0,
  }).format(BigInt(whole!));
  const separator =
    new Intl.NumberFormat()
      .formatToParts(1.1)
      .find((part) => part.type === "decimal")?.value ?? ".";
  return `${pico < 0n ? "-" : ""}${grouped}${fraction ? separator + fraction : ""}`;
}
export function hasCredits(value: string | number | null | undefined): boolean {
  return (
    value != null &&
    (typeof value === "string" ? parseCredits(value) > 0n : value > 0)
  );
}

/** Compact headlines round exact integer picocredits to one shown decimal. */
export function formatCompactCredits(value: string): string {
  const pico = parseCredits(value);
  const magnitude = pico < 0n ? -pico : pico;
  const units = [
    [1_000_000_000_000n, "T"],
    [1_000_000_000n, "B"],
    [1_000_000n, "M"],
    [1_000n, "K"],
  ] as const;
  for (const [scale, suffix] of units) {
    const divisor = scale * CREDIT_SCALE;
    if (magnitude >= divisor) {
      const tenths = (magnitude * 10n + divisor / 2n) / divisor;
      const digits = formatExactCredits(
        decimalCredits((tenths * CREDIT_SCALE) / 10n),
        1,
      );
      return `${pico < 0n ? "-" : ""}${digits}${suffix}`;
    }
  }
  return formatExactCredits(value);
}
