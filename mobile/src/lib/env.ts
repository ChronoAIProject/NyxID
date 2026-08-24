/**
 * True when running a local development build (pnpm run ios / pnpm run android).
 * False for release builds (build:ios, build:ios:testflight, EAS).
 *
 * Set via EXPO_PUBLIC_DEV_MODE=true in the dev script (package.json).
 */
export const IS_DEV_BUILD = process.env.EXPO_PUBLIC_DEV_MODE === "true";

/**
 * Trusted URL inputs embedded by app.config.ts. The device-login QR parser
 * consumes these values directly so a scanned URL can never choose its own
 * backend or expand the set of accepted frontend origins.
 */
export const NYXID_API_BASE_URL = process.env.EXPO_PUBLIC_API_BASE_URL ?? "";
export const NYXID_FRONTEND_URL = process.env.EXPO_PUBLIC_FRONTEND_URL ?? "";
export const NYXID_UNIVERSAL_LINK_HOST =
  process.env.EXPO_PUBLIC_UNIVERSAL_LINK_HOST ?? "";
export const NYXID_APP_SCHEME = process.env.EXPO_PUBLIC_APP_SCHEME ?? "nyxid";

/**
 * Comma-separated list of emails allowed to use the mobile app.
 * If empty or unset, all authenticated users are allowed.
 */
const ALLOWED_EMAILS_RAW = process.env.EXPO_PUBLIC_ALLOWED_EMAILS ?? "";
export const ALLOWED_EMAILS: string[] = ALLOWED_EMAILS_RAW
  ? ALLOWED_EMAILS_RAW.split(",").map((e: string) => e.trim().toLowerCase()).filter(Boolean)
  : [];

export function isEmailAllowed(email: string): boolean {
  if (ALLOWED_EMAILS.length === 0) return true;
  return ALLOWED_EMAILS.includes(email.trim().toLowerCase());
}
