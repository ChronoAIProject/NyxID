import { type ClassValue, clsx } from "clsx";
import { twMerge } from "tailwind-merge";

export function cn(...inputs: ClassValue[]) {
  return twMerge(clsx(inputs));
}

export function formatDate(dateStr: string | null | undefined): string {
  if (!dateStr) return "N/A";
  const date = new Date(dateStr);
  if (Number.isNaN(date.getTime())) return "N/A";
  return new Intl.DateTimeFormat("en-US", {
    month: "short",
    day: "numeric",
    year: "numeric",
  }).format(date);
}

export function formatDateTime(dateStr: string | null | undefined): string {
  if (!dateStr) return "N/A";
  const date = new Date(dateStr);
  if (Number.isNaN(date.getTime())) return "N/A";
  return new Intl.DateTimeFormat("en-US", {
    month: "short",
    day: "numeric",
    year: "numeric",
    hour: "2-digit",
    minute: "2-digit",
    hour12: false,
  }).format(date);
}

export function formatRelativeTime(dateStr: string | null | undefined): string {
  if (!dateStr) return "N/A";
  const date = new Date(dateStr);
  if (Number.isNaN(date.getTime())) return "N/A";
  const now = new Date();
  const diffMs = now.getTime() - date.getTime();
  const diffSeconds = Math.floor(diffMs / 1000);
  const diffMinutes = Math.floor(diffSeconds / 60);
  const diffHours = Math.floor(diffMinutes / 60);
  const diffDays = Math.floor(diffHours / 24);

  if (diffSeconds < 60) return "just now";
  if (diffMinutes < 60) return `${String(diffMinutes)}m ago`;
  if (diffHours < 24) return `${String(diffHours)}h ago`;
  if (diffDays < 7) return `${String(diffDays)}d ago`;
  return formatDate(dateStr);
}

/**
 * Format a timestamp relative to now, supporting both past and future times.
 *
 * Past times render like `formatRelativeTime` (e.g. `15m ago`). Future times
 * render as `in 15m`, `in 3h`, `in 2d`, or an absolute date for anything
 * farther than a week out. Use this for "expires at" timestamps and similar
 * fields where the value can legitimately sit in the future.
 */
export function formatTimeDistance(
  dateStr: string | null | undefined,
): string {
  if (!dateStr) return "N/A";
  const date = new Date(dateStr);
  if (Number.isNaN(date.getTime())) return "N/A";
  const diffMs = date.getTime() - Date.now();
  const absMs = Math.abs(diffMs);
  const absSeconds = Math.floor(absMs / 1000);
  const absMinutes = Math.floor(absSeconds / 60);
  const absHours = Math.floor(absMinutes / 60);
  const absDays = Math.floor(absHours / 24);

  const isFuture = diffMs > 0;
  const suffix = isFuture ? "" : " ago";
  const prefix = isFuture ? "in " : "";

  if (absSeconds < 60) return isFuture ? "in a moment" : "just now";
  if (absMinutes < 60) return `${prefix}${String(absMinutes)}m${suffix}`;
  if (absHours < 24) return `${prefix}${String(absHours)}h${suffix}`;
  if (absDays < 7) return `${prefix}${String(absDays)}d${suffix}`;
  return formatDate(dateStr);
}

export function maskApiKey(keyPrefix: string): string {
  return `${keyPrefix}${"*".repeat(24)}`;
}

export function isPastTimestamp(dateStr: string | null | undefined): boolean {
  if (!dateStr) return false;
  const date = new Date(dateStr);
  if (Number.isNaN(date.getTime())) return false;
  return date.getTime() <= Date.now();
}

export async function copyToClipboard(text: string): Promise<void> {
  if (!navigator.clipboard) {
    throw new Error("Clipboard API is not available in this browser context");
  }
  await navigator.clipboard.writeText(text);
}

export function sanitizeAvatarUrl(
  url: string | null | undefined,
): string | null {
  if (!url) return null;
  try {
    const parsed = new URL(url);
    if (parsed.protocol === "https:" || parsed.protocol === "http:") {
      return url;
    }
    return null;
  } catch {
    return null;
  }
}
