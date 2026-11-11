import { activeDomains } from "../domains";

/** The active API base, selected once per launch (see `lib/domains.ts`). */
export function getApiBaseUrl(): string {
  const apiBaseUrl = activeDomains().apiBaseUrl;
  if (!apiBaseUrl) {
    throw new Error("API base URL is not configured. Set the mobile build's API_BASE_URL.");
  }
  return apiBaseUrl;
}

export function buildApiUrl(path: string): string {
  if (/^(?:[a-z][a-z0-9+.-]*:|\/\/)/i.test(path)) {
    throw new Error("API endpoints must be relative paths.");
  }
  return `${getApiBaseUrl()}${path.startsWith("/") ? path : `/${path}`}`;
}
