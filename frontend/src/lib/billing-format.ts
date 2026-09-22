export function formatCredits(value: number): string {
  return `${formatNumber(value)} credits`;
}

export function formatNumber(value: number): string {
  return new Intl.NumberFormat().format(value);
}

export function formatEstimatedCredits(
  value: number | null | undefined,
): string {
  if (value === null || value === undefined) {
    return "-";
  }
  return `${new Intl.NumberFormat(undefined, {
    maximumFractionDigits: 6,
  }).format(value / 1_000_000)} credits`;
}
