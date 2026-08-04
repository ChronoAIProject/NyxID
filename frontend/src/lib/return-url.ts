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
