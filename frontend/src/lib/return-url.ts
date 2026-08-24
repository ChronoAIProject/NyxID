/** Resolve a return_to value only when it targets an allowed browser origin. */
function configuredBackendOrigin(): string | null {
  const configuredUrl = (
    (import.meta.env.VITE_BACKEND_URL as string | undefined) ??
    (import.meta.env.VITE_API_URL as string | undefined) ??
    ""
  ).trim();

  if (!configuredUrl) return null;

  try {
    const url = new URL(configuredUrl);
    return url.protocol === "http:" || url.protocol === "https:"
      ? url.origin
      : null;
  } catch {
    return null;
  }
}

export function resolveTrustedAuthReturnTo(
  value: string | undefined,
): string | null {
  if (!value) return null;

  let url: URL;
  try {
    url = new URL(value, window.location.origin);
  } catch {
    return null;
  }

  if (url.protocol !== "http:" && url.protocol !== "https:") {
    return null;
  }

  const frontendOrigin = window.location.origin;
  const backendOrigin = configuredBackendOrigin();
  if (url.origin !== frontendOrigin && url.origin !== backendOrigin) {
    return null;
  }

  return url.href;
}
