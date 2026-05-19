/**
 * True when running a local development build (pnpm run ios / pnpm run android).
 * False for release builds (build:ios, build:ios:testflight, EAS).
 *
 * Set via EXPO_PUBLIC_DEV_MODE=true in the dev script (package.json).
 */
export const IS_DEV_BUILD = process.env.EXPO_PUBLIC_DEV_MODE === "true";

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
