/** CRM currencies use integer hundredths. Keep display independent of float division. */
export function currencyDecimal(minor: number): string {
  if (!Number.isSafeInteger(minor)) throw new Error("Invalid monetary amount");
  const amount = BigInt(minor);
  const absolute = amount < 0n ? -amount : amount;
  return `${amount < 0n ? "-" : ""}${absolute / 100n}.${String(absolute % 100n).padStart(2, "0")}`;
}

export function formatCrmMoney(
  minor: number,
  currency: string,
  locale?: string,
): string {
  if (!Number.isSafeInteger(minor)) return "Unavailable amount";
  const amount = BigInt(minor);
  const absolute = amount < 0n ? -amount : amount;
  const whole = absolute / 100n;
  const signed = amount < 0n ? (whole === 0n ? -0 : -whole) : whole;
  const fraction = String(absolute % 100n).padStart(2, "0");
  return new Intl.NumberFormat(locale, {
    style: "currency",
    currency,
    minimumFractionDigits: 2,
    maximumFractionDigits: 2,
  })
    .formatToParts(signed)
    .map((part) => (part.type === "fraction" ? fraction : part.value))
    .join("");
}
