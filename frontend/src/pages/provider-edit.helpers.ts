// `splitScopes` is functionally identical to the implementation in
// provider-list.helpers.ts, so it is re-exported here rather than duplicated.
export { splitScopes } from "./provider-list.helpers";

export const PROVIDER_TYPE_LABELS: Readonly<Record<string, string>> = {
  oauth2: "OAuth 2.0",
  api_key: "API Key",
  device_code: "Device Code",
  telegram_widget: "Telegram Widget",
};

export function stripEmptyStrings<T extends Record<string, unknown>>(
  obj: T,
): Record<string, unknown> {
  return Object.fromEntries(
    Object.entries(obj).filter(([, v]) => v !== "" && v !== undefined),
  );
}
