import { afterEach, describe, expect, it, vi } from "vitest";
import {
  authHref,
  injectRedditDefaultCode,
  siteHref,
  legacyDestination,
} from "./auth-links";

afterEach(() => {
  vi.restoreAllMocks();
  window.history.replaceState({}, "", "/");
});

describe("website authentication entry parity", () => {
  it("preserves invitation, trusted return and campaign query through public navigation and both auth entries", () => {
    const query = "?code=INVITE-42&return_to=%2Fkeys&utm_source=campaign";
    window.history.replaceState({}, "", `/${query}`);
    window.history.replaceState({}, "", siteHref("/solutions"));
    expect(window.location.search).toBe(query);
    expect(authHref("login")).toBe(`/login${query}`);
    expect(authHref("register")).toBe(`/register${query}`);
  });
  it("does not replace an explicit invitation on a Reddit referral", () => {
    window.history.replaceState({}, "", "/?code=EXPLICIT&utm_source=reddit");
    injectRedditDefaultCode();
    expect(new URLSearchParams(window.location.search).get("code")).toBe(
      "EXPLICIT",
    );
  });
  it("retains the existing Reddit fallback without dropping return, hash or router history state", () => {
    window.history.replaceState(
      { key: "router" },
      "",
      "/?utm_source=reddit&return_to=%2Fkeys#beta",
    );
    injectRedditDefaultCode();
    const params = new URLSearchParams(window.location.search);
    expect(params.get("code")).toBe("NYX-QJGT6NHN");
    expect(params.get("return_to")).toBe("/keys");
    expect(window.location.hash).toBe("#beta");
    expect(window.history.state).toEqual({ key: "router" });
  });
  it("does not inject attribution for non-Reddit visits or lookalike domains", () => {
    vi.spyOn(document, "referrer", "get").mockReturnValue(
      "https://reddit.com.attacker.test/",
    );
    injectRedditDefaultCode();
    expect(window.location.search).toBe("");
  });
  it("merges target display parameters while keeping authentication context", () => {
    expect(siteHref("/?for=dev#try", "?code=INVITE&for=business")).toBe(
      "/?code=INVITE&for=dev#try",
    );
  });
});

it("only migrates recognized prototype paths and cannot become an external redirect", () => {
  for (const hash of [
    "#//example.org",
    "#/\\example.org",
    "#/https://example.org",
    "#/unknown",
    "#/solutions/../login",
    "#/solutions%2Fhelp-desk",
  ]) {
    expect(legacyDestination(hash, "?code=INVITE")).toBeNull();
  }
  expect(
    legacyDestination("#/solutions/restaurant-manager", "?code=INVITE"),
  ).toBe("/solutions/restaurant-manager?code=INVITE");
  expect(
    legacyDestination("#/start?t=restaurant-manager", "?code=INVITE"),
  ).toBe("/register?code=INVITE");
  expect(legacyDestination("#/login", "?return_to=%2Fkeys")).toBe(
    "/login?return_to=%2Fkeys",
  );
});
