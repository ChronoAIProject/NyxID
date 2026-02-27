import { isNative } from "./platform";

// ===== Backend API Origin =====
// Web: relative path (Vite proxy / same-origin in prod)
// Native: absolute URL (no proxy in WebView)
export const API_ORIGIN = isNative
  ? (import.meta.env.VITE_API_URL as string | undefined) ?? ""
  : "";

export const API_BASE = `${API_ORIGIN}/api/v1`;

// ===== Trusted Redirect Origins (open-redirect prevention) =====
export const BACKEND_ORIGIN = (
  (import.meta.env.VITE_API_URL as string | undefined) ?? ""
).replace(/\/+$/, "");

export const FRONTEND_ORIGIN = typeof window !== "undefined"
  ? window.location.origin
  : "";

export function isTrustedRedirect(url: string): boolean {
  return (
    url.startsWith(FRONTEND_ORIGIN + "/") ||
    (BACKEND_ORIGIN !== "" && url.startsWith(BACKEND_ORIGIN + "/"))
  );
}
