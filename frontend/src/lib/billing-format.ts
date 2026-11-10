import { getFormattingLocale } from "@/lib/formatting-locale";
import { translateUi } from "@/i18n/use-ui-translation";
import { credits } from "./billing-display";
export function formatCredits(value: number): string {
  return translateUi("{{value}} credits", { value: formatNumber(value) });
}

export function formatNumber(value: number): string {
  return new Intl.NumberFormat(getFormattingLocale()).format(value);
}

export function formatEstimatedCredits(
  value: string | number | null | undefined,
): string {
  if (value === null || value === undefined) {
    return "-";
  }
  return translateUi("{{value}} credits", { value: credits(value) });
}
