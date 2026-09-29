import { credits } from "./billing-display";
export function formatCredits(value: number): string {
  return `${formatNumber(value)} credits`;
}

export function formatNumber(value: number): string {
  return new Intl.NumberFormat().format(value);
}

export function formatEstimatedCredits(
  value: string | number | null | undefined,
): string {
  if (value === null || value === undefined) {
    return "-";
  }
  return `${credits(value)} credits`;
}
