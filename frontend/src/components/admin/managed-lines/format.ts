import { translateUi } from "@/i18n/use-ui-translation";
import { getFormattingLocale } from "@/lib/formatting-locale";

export function when(value: string | null | undefined): string {
  if (!value) return "—";
  const parsed = new Date(value);
  return Number.isNaN(parsed.getTime())
    ? value
    : parsed.toLocaleString(getFormattingLocale());
}

export function lineStateLabel(state: string, suspension: string | null): string {
  if (state === "suspended") {
    switch (suspension) {
      case "admin":
        return translateUi("Suspended by an administrator");
      case "unpaid":
        return translateUi("Suspended: unpaid");
      case "eligibility_revoked":
        return translateUi("Suspended: eligibility revoked");
      default:
        return translateUi("Suspended");
    }
  }
  switch (state) {
    case "available":
      return translateUi("Available");
    case "claimed":
      return translateUi("Claimed");
    case "releasing":
      return translateUi("Releasing");
    case "cooldown":
      return translateUi("Cooling down");
    case "pending_reassignment":
      return translateUi("Awaiting reassignment approval");
    case "retired":
      return translateUi("Retired");
    default:
      return state;
  }
}

export function failure(error: unknown, fallback: string): string {
  return error instanceof Error && error.message ? error.message : fallback;
}
