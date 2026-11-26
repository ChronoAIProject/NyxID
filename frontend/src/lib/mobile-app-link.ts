const DEFAULT_MOBILE_APP_LINK = "https://nyxid.onelink.me/REzJ/dql9w8fx";

/** Override at build time with a provisioned HTTPS download URL. */
export function resolveMobileAppLink(value: string | undefined): string {
  try {
    const url = new URL(value?.trim() ?? "");
    if (url.protocol === "https:") return url.href;
  } catch { /* Keep the existing download destination for invalid overrides. */ }
  return DEFAULT_MOBILE_APP_LINK;
}

export const MOBILE_APP_LINK = resolveMobileAppLink(import.meta.env.VITE_MOBILE_APP_URL);
