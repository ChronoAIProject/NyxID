import { NYXID_API_BASE_URL } from "../domains";

/** The active API base, selected once per launch (see `lib/domains.ts`). */
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
