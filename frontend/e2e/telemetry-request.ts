/** Classify requests observed by the production egress test. */
export function isTelemetryRequest(requestUrl: string, configuredHost: string): boolean {
  const url = new URL(requestUrl);
  return url.hostname === "posthog.com"
    || url.hostname.endsWith(".posthog.com")
    || url.origin === new URL(configuredHost).origin;
}

/** Retain every cross-origin data request for the egress assertion, including lookalikes. */
export function observeTelemetryEgress(
  requests: string[], requestUrl: string, resourceType: string, appOrigin: string, configuredHost: string,
): void {
  if (isTelemetryRequest(requestUrl, configuredHost)
    || ((resourceType === "fetch" || resourceType === "xhr") && new URL(requestUrl).origin !== appOrigin)) {
    requests.push(requestUrl);
  }
}

export function hasOnlyConfiguredEgress(requests: string[], configuredHost: string): boolean {
  return requests.every((url) => new URL(url).origin === new URL(configuredHost).origin);
}
