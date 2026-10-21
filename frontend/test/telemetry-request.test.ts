import { describe, expect, it } from "vitest";
import { hasOnlyConfiguredEgress, isTelemetryRequest, observeTelemetryEgress } from "../e2e/telemetry-request";

describe("production telemetry request observation", () => {
  it.each([
    "https://posthog.com/i/v0/e/",
    "https://us.i.posthog.com/i/v0/e/",
    "https://us-assets.i.posthog.com/static/array.js",
    "https://analytics.example/i/v0/e/",
    "https://ANALYTICS.EXAMPLE:443/i/v0/e/",
  ])("observes the actual vendor domains or configured origin: %s", (url) => {
    expect(isTelemetryRequest(url, "https://analytics.example")).toBe(true);
  });

  it.each([
    "https://notposthog.com/i/v0/e/",
    "https://posthog.com.attacker.example/i/v0/e/",
    "https://analytics.example.attacker.example/i/v0/e/",
    "https://attacker.example/posthog.com",
    "https://posthog.com@attacker.example/i/v0/e/",
    "https://analytics.example@attacker.example/i/v0/e/",
    "https://analytics.example:444/i/v0/e/",
    "http://analytics.example/i/v0/e/",
  ])("does not confuse substrings, userinfo or other origins: %s", (url) => {
    expect(isTelemetryRequest(url, "https://analytics.example")).toBe(false);
  });

  it("continues observing unexpected insecure vendor requests for the egress assertion", () => {
    expect(isTelemetryRequest("http://us.i.posthog.com/i/v0/e/", "https://analytics.example"))
      .toBe(true);
  });

  it.each([
    "https://notposthog.com/i/v0/e/",
    "https://posthog.com.attacker.example/i/v0/e/",
    "https://analytics.example.attacker.example/i/v0/e/",
    "https://attacker.example/posthog.com",
    "https://connect.facebook.net/allowed-by-csp",
  ])("retains unexpected data egress and fails the final assertion: %s", (url) => {
    for (const resource of ["fetch", "xhr"]) {
      const requests: string[] = [];
      observeTelemetryEgress(requests, url, resource, "https://app.example", "https://analytics.example");
      expect(requests).toEqual([url]);
      expect(hasOnlyConfiguredEgress(requests, "https://analytics.example")).toBe(false);
    }
  });

  it("collects configured data egress and vendor scripts while allowing ordinary app data and fonts", () => {
    const requests: string[] = [];
    for (const [url, resource] of [
      ["https://app.example/api/v1/public/config", "fetch"],
      ["https://fonts.gstatic.com/font.woff2", "font"],
      ["https://analytics.example/i/v0/e/", "fetch"],
      ["https://us-assets.i.posthog.com/lazy.js", "script"],
    ] as const) observeTelemetryEgress(requests, url, resource, "https://app.example", "https://analytics.example");
    expect(requests).toEqual(["https://analytics.example/i/v0/e/", "https://us-assets.i.posthog.com/lazy.js"]);
    expect(hasOnlyConfiguredEgress(requests.slice(0, 1), "https://analytics.example")).toBe(true);
    expect(hasOnlyConfiguredEgress(requests, "https://analytics.example")).toBe(false);
  });
});
