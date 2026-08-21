import { ApiError } from "@/lib/api-client";

const SECRET_VALUE =
  /(?:Bearer\s+\S+|nyxid_(?:ag_)?[A-Za-z0-9_-]{16,})/i;

const FORBIDDEN_READ_BACK_FIELDS = new Set([
  "accesstoken",
  "apikey",
  "authorization",
  "cookie",
  "cookies",
  "credential",
  "credentials",
  "fullkey",
  "keyhash",
  "password",
  "rawbody",
  "rawupstreambody",
  "refreshtoken",
  "secret",
  "secrets",
  "token",
  "passphrase",
  "usercode",
  "devicecode",
]);

/** Recursively apply the assistant reader's secret-shape policy locally. */
export function assertSecretFreeReadBack(value: unknown): void {
  if (typeof value === "string" && SECRET_VALUE.test(value)) {
    throw new Error("NyxID returned secret-bearing verification data.");
  }
  if (Array.isArray(value)) {
    for (const entry of value) assertSecretFreeReadBack(entry);
    return;
  }
  if (!value || typeof value !== "object") return;
  for (const [key, entry] of Object.entries(value)) {
    const normalized = key
      .replace(/[^A-Za-z0-9]/g, "")
      .toLocaleLowerCase("en-US");
    if (FORBIDDEN_READ_BACK_FIELDS.has(normalized)) {
      throw new Error("NyxID returned secret-bearing verification data.");
    }
    assertSecretFreeReadBack(entry);
  }
}

/** Reject secret material supplied by the assistant instead of the browser. */
export function assertNoSensitiveActionParams(value: unknown): void {
  try {
    assertSecretFreeReadBack(value);
  } catch {
    throw new Error("Sensitive values must be entered in the NyxID browser dialog, not supplied by chat.");
  }
}

export function errorMessage(caught: unknown, fallback: string): string {
  if (caught instanceof ApiError) return caught.message;
  if (caught instanceof Error && caught.message.trim()) return caught.message;
  return fallback;
}

export function isNotFound(caught: unknown): boolean {
  return caught instanceof ApiError && caught.status === 404;
}

export function isNewerTimestamp(before: string, after: string): boolean {
  const beforeMs = Date.parse(before);
  const afterMs = Date.parse(after);
  return Number.isFinite(beforeMs) && Number.isFinite(afterMs) && afterMs > beforeMs;
}

export const SECRET_VALUE_PATTERN = SECRET_VALUE;
