import { z } from "zod";

const date = z.iso.date();
const timestamp = z.iso.datetime({ offset: true });

export function normalizeAgentKeyExpiry(input: string): string {
  const value = input.trim();
  if (date.safeParse(value).success) return `${value}T23:59:59Z`;
  const normalized = value.replace("t", "T").replace(/z$/, "Z");
  if (
    /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d+)?(?:Z|[+-]\d{2}:\d{2})$/.test(normalized) &&
    timestamp.safeParse(normalized).success
  ) return normalized;
  throw new Error("Enter an expiry as YYYY-MM-DD or an RFC 3339 timestamp with a timezone.");
}
