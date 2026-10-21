import { describe, expect, it } from "vitest";
import { hasOnlyRelayEgress, isTelemetryRequest, observeTelemetryEgress } from "../e2e/telemetry-request";

const app = "https://app.example";
const relay = `${app}/api/v1/users/me/analytics/events`;

describe("production telemetry request observation", () => {
  it.each([relay, "https://APP.EXAMPLE:443/api/v1/users/me/analytics/events"])("recognizes only the same-origin relay: %s", (url) => {
    expect(isTelemetryRequest(url, app)).toBe(true);
  });

  it.each([
    "https://us.i.posthog.com/i/v0/e/", "https://eu.i.posthog.com/i/v0/e/",
    "https://analytics.example/api/v1/users/me/analytics/events",
    "https://app.example.attacker.example/api/v1/users/me/analytics/events",
    "https://app.example@attacker.example/api/v1/users/me/analytics/events",
    "https://user@app.example/api/v1/users/me/analytics/events",
    "https://app.example:444/api/v1/users/me/analytics/events",
    "http://app.example/api/v1/users/me/analytics/events",
    `${relay}/`, `${relay}?secret=fixture`, `${relay}#fixture`,
    `${app}/api/v1/users/me/preferences/analytics`, `${app}/api/v1/public/config`,
  ])("does not admit vendors, lookalikes or other API requests: %s", (url) => {
    expect(isTelemetryRequest(url, app)).toBe(false);
    expect(hasOnlyRelayEgress([url], app)).toBe(false);
  });

  it.each([
    "https://notposthog.com/i/v0/e/", "https://posthog.com.attacker.example/i/v0/e/",
    "https://connect.facebook.net/allowed-by-csp", "http://us.i.posthog.com/i/v0/e/",
    "https://analytics.example/i/v0/e/", "https://us-assets.i.posthog.com/lazy.js",
  ])("retains unexpected external data and scripts: %s", (url) => {
    for (const resource of ["fetch", "xhr", "script", "eventsource", "websocket", "ping", "other"]) {
      const requests: string[] = [];
      observeTelemetryEgress(requests, url, resource, app);
      expect(requests).toEqual([url]);
      expect(hasOnlyRelayEgress(requests, app)).toBe(false);
    }
  });

  it("collects relay delivery while allowing ordinary app assets/data and fonts", () => {
    const requests: string[] = [];
    for (const [url, resource] of [
      [`${app}/api/v1/public/config`, "fetch"],
      [`${app}/assets/app.js`, "script"],
      ["https://fonts.gstatic.com/font.woff2", "font"],
      ["https://fonts.googleapis.com/font.css", "stylesheet"],
      [relay, "fetch"],
    ] as const) observeTelemetryEgress(requests, url, resource, app);
    expect(requests).toEqual([relay]);
    expect(hasOnlyRelayEgress(requests, app)).toBe(true);
  });
});
