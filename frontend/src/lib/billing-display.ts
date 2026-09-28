import type { BillingUsagePeriod, BillingUsageRow } from "@/schemas/billing";
import type { CatalogEntry } from "@/types/keys";

export type BillingCatalog = readonly Pick<
  CatalogEntry,
  "slug" | "name" | "inference"
>[];

export const periods: Record<BillingUsagePeriod, string> = {
  "24h": "Last 24 hours",
  "7d": "Last 7 days",
  "30d": "Last 30 days",
  "90d": "Last 90 days",
  all: "All time",
};
export const number = (value: number) =>
  new Intl.NumberFormat(undefined, { maximumFractionDigits: 6 }).format(value);
export const credits = (micros: number | null | undefined) =>
  micros == null ? "Unavailable" : number(micros / 1_000_000);
export const compact = (value: number) =>
  new Intl.NumberFormat(undefined, {
    notation: "compact",
    maximumFractionDigits: 1,
  }).format(value);
export const timestamp = (value: string) => new Date(value).toLocaleString();
export const serviceName = (catalog: BillingCatalog, slug?: string | null) =>
  catalog.find((entry) => entry.slug === slug)?.name ??
  (slug ? slug.replace(/[-_]+/g, " ") : "Unavailable service");
export const serviceCategory = (catalog: BillingCatalog, slug: string) => {
  const entry = catalog.find((service) => service.slug === slug);
  return entry
    ? entry.inference
      ? "AI models"
      : "Connected apps"
    : "Other services";
};

export function total(
  rows: readonly BillingUsageRow[],
  field:
    | "estimated_credits_micros"
    | "wallet_credits_micros"
    | "grant_credits_micros"
    | "allowance_credits_micros",
): number | null {
  if (rows.some((row) => row[field] == null)) return null;
  return rows.reduce((sum, row) => sum + (row[field] ?? 0), 0);
}

const DAY_MS = 86_400_000;
/** Expiry dates this close also show a relative hint. */
const SOON_DAYS = 14;

/** e.g. "Oct 31 · in 5 days"; the year only when it is not this year. */
export function expiryLabel(expiresAt: string | null | undefined, now: number) {
  if (!expiresAt) return "No expiry";
  const date = new Date(expiresAt);
  const text = date.toLocaleDateString(undefined, {
    month: "short",
    day: "numeric",
    ...(date.getFullYear() !== new Date(now).getFullYear()
      ? { year: "numeric" }
      : {}),
  });
  const days = Math.ceil((date.getTime() - now) / DAY_MS);
  if (days > SOON_DAYS) return text;
  const relative =
    days <= 0
      ? "today"
      : new Intl.RelativeTimeFormat(undefined, { numeric: "auto" }).format(
          days,
          "day",
        );
  return `${text} · ${relative}`;
}
