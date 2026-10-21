/** Only this same-origin endpoint delivers browser product events. */
export function isTelemetryRequest(requestUrl: string, appOrigin: string): boolean {
  const url = new URL(requestUrl);
  return url.origin === new URL(appOrigin).origin
    && url.pathname === "/api/v1/users/me/analytics/events"
    && !url.username && !url.password && !url.search && !url.hash;
}

/** Retain relay delivery and all external data/script attempts, even lookalikes.
 * Requests blocked by CSP are checked separately by productionPolicy.
 */
export function observeTelemetryEgress(
  requests: string[], requestUrl: string, resourceType: string, appOrigin: string,
): void {
  if (isTelemetryRequest(requestUrl, appOrigin)
    || (["fetch", "xhr", "script", "eventsource", "websocket", "ping", "other"].includes(resourceType)
      && new URL(requestUrl).origin !== new URL(appOrigin).origin)) {
    requests.push(requestUrl);
  }
}

export function hasOnlyRelayEgress(requests: string[], appOrigin: string): boolean {
  return requests.every((url) => isTelemetryRequest(url, appOrigin));
}
