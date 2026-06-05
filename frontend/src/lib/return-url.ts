/** Trusted origins for auth return_to redirect validation. */
const BACKEND_URL = (
  (import.meta.env.VITE_BACKEND_URL as string | undefined) ??
  (import.meta.env.VITE_API_URL as string | undefined) ??
  ""
).replace(/\/+$/, "");

const FRONTEND_ORIGIN = window.location.origin;

export function isTrustedAuthReturnTo(value: string | undefined): value is string {
  return Boolean(
    value &&
      (value.startsWith(FRONTEND_ORIGIN + "/") ||
        value.startsWith(BACKEND_URL + "/")),
  );
}

export function getSafeCredentialAcceptReturnTo(
  value: string | undefined,
): string | null {
  if (!value || value.length > 2048) {
    return null;
  }

  if (value.startsWith("/")) {
    if (value.startsWith("//") || value.startsWith("/\\")) {
      return null;
    }
    try {
      const url = new URL(value, FRONTEND_ORIGIN);
      if (url.origin !== FRONTEND_ORIGIN) {
        return null;
      }
      return `${url.pathname}${url.search}${url.hash}`;
    } catch {
      return null;
    }
  }

  try {
    const url = new URL(value);
    if (url.origin !== FRONTEND_ORIGIN) {
      return null;
    }
    return url.href;
  } catch {
    return null;
  }
}
