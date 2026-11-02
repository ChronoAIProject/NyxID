function resolveApiBaseUrl(configuredUrl: string | undefined): string {
  const normalized = (configuredUrl ?? "").trim().replace(/\/+$/, "");
  return normalized.endsWith("/mobile")
    ? normalized.slice(0, -"/mobile".length)
    : normalized;
}

export const NYXID_API_BASE_URL = resolveApiBaseUrl(process.env.EXPO_PUBLIC_API_BASE_URL);

export function getApiBaseUrl(): string {
  if (!NYXID_API_BASE_URL) {
    throw new Error("API base URL is not configured. Set the mobile build's API_BASE_URL.");
  }
  return NYXID_API_BASE_URL;
}

export function buildApiUrl(path: string): string {
  if (/^(?:[a-z][a-z0-9+.-]*:|\/\/)/i.test(path)) {
    throw new Error("API endpoints must be relative paths.");
  }
  return `${getApiBaseUrl()}${path.startsWith("/") ? path : `/${path}`}`;
}
