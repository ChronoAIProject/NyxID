type FirstPartyHosts = {
  apiBaseUrls: readonly string[];
  frontendUrls: readonly string[];
  universalLinkHost?: string;
};

/** Association host choices do not change routing. The same trusted origins
 * supply native pages and explicit Web fallbacks under either host rollout. */
export function nativeWebFallback(
  url: string,
  hosts: FirstPartyHosts,
  frontendUrl: string,
): string | null {
  try {
    const parsed = new URL(url);
    if (
      parsed.protocol !== "https:" ||
      parsed.username ||
      parsed.password ||
      parsed.hostname === hosts.universalLinkHost?.toLowerCase()
    )
      return null;
    const origins = (values: readonly string[]) =>
      values.flatMap((value) => {
        try {
          return [new URL(value).origin];
        } catch {
          return [];
        }
      });
    const api = origins(hosts.apiBaseUrls).includes(parsed.origin);
    if (!api && !origins(hosts.frontendUrls).includes(parsed.origin))
      return null;
    const destination = api ? new URL(frontendUrl) : parsed;
    if (
      destination.protocol !== "https:" ||
      destination.username ||
      destination.password
    )
      return null;
    return `${destination.origin}${parsed.pathname}${parsed.search}`;
  } catch {
    return null;
  }
}

export function deduplicatedWebFallback(
  open: (url: string) => Promise<unknown>,
) {
  const delivered = new Map<string, number>();
  return (url: string) => {
    const now = Date.now();
    for (const [key, time] of delivered)
      if (now - time > 30_000) delivered.delete(key);
    if (delivered.has(url)) return;
    if (delivered.size >= 64) delivered.delete(delivered.keys().next().value!);
    delivered.set(url, now);
    void open(url).catch(() => {
      delivered.delete(url);
    });
  };
}
