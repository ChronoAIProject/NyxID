const MICROS_PER_CREDIT = 1_000_000n;

/**
 * Formats an exact microcredit string without passing it through a JS
 * Number. Fractional precision is retained up to the six digits supported by
 * the billing ledger and integer grouping follows the user's locale.
 */
export function formatMicrocredits(
  value: string,
  options: { readonly suffix?: boolean; readonly locale?: string } = {},
): string {
  const { suffix = true, locale } = options;
  let micros = BigInt(value);
  const negative = micros < 0n;
  if (negative) micros = -micros;
  const whole = micros / MICROS_PER_CREDIT;
  const fraction = (micros % MICROS_PER_CREDIT)
    .toString()
    .padStart(6, "0")
    .replace(/0+$/, "");
  const formatter = new Intl.NumberFormat(locale);
  // Keep locale sign/bidi literals, including the sign of a fractional -0.
  const parts = formatter.formatToParts(
    negative ? (whole === 0n ? -0 : -whole) : whole,
  );
  const lastInteger = parts.reduce(
    (last, part, index) => (part.type === "integer" ? index : last),
    -1,
  );
  const localizedFraction = fraction
    ? decimalSeparator(locale) + localizeDigits(fraction, locale)
    : "";
  return (
    parts
      .map(
        ({ value }, index) =>
          value + (index === lastInteger ? localizedFraction : ""),
      )
      .join("") + (suffix ? " credits" : "")
  );
}

export function formatCreditUnits(
  value: string,
  options: { readonly suffix?: boolean; readonly locale?: string } = {},
): string {
  const { suffix = true, locale } = options;
  const amount = BigInt(value);
  return (
    new Intl.NumberFormat(locale).format(amount) + (suffix ? " credits" : "")
  );
}

function decimalSeparator(locale?: string): string {
  const part = new Intl.NumberFormat(locale)
    .formatToParts(1.1)
    .find(({ type }) => type === "decimal");
  return part?.value ?? ".";
}

function localizeDigits(digits: string, locale?: string): string {
  const formatter = new Intl.NumberFormat(locale, { useGrouping: false });
  const localized = Array.from({ length: 10 }, (_, digit) =>
    formatter.format(digit),
  );
  return digits.replace(/\d/g, (digit) => localized[Number(digit)]!);
}
