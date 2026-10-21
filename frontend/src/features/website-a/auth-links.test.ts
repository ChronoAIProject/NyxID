import { afterEach, describe, expect, it, vi } from "vitest";
import {
  authHref,
  injectRedditDefaultCode,
  siteHref,
  legacyDestination,
} from "./auth-links";

afterEach(() => {
  vi.restoreAllMocks();
  vi.unstubAllEnvs();
  vi.resetModules();
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

describe("legacy authentication fragment context", () => {
  it("hands an accepted legacy solution to auth with the invitation and selected target intact", async () => {
    vi.stubEnv(
      "VITE_WEBSITE_SOLUTION_ACTIVATIONS",
      JSON.stringify({
        "help-desk": {
          target: "/assistant?setup=help-desk",
          acceptance: "review/help-desk",
        },
      }),
    );
    vi.resetModules();
    const { legacyDestination: resolve } = await import("./auth-links");
    const destination = new URL(
      resolve("#/start?t=help-desk", "?code=NYX-ABCDEF12&return_to=%2Fkeys")!,
      window.location.origin,
    );
    expect(destination.pathname).toBe("/login");
    expect(destination.searchParams.get("code")).toBe("NYX-ABCDEF12");
    expect(destination.searchParams.get("return_to")).toBe(
      "/assistant?setup=help-desk",
    );
  });
  it.each([
    ["/login", "/login"],
    ["/signup", "/register"],
  ])("merges %s fragment parameters with the outer query", (legacy, target) => {
    const returnTo = "/oauth/authorize?client_id=web&scope=openid";
    const fragment = new URLSearchParams({
      code: "FRAGMENT-INVITE",
      return_to: returnTo,
      lang: "zh",
      utm_source: "fragment",
    });
    const destination = legacyDestination(
      `#${legacy}?${fragment}`,
      "?code=OUTER&return_to=%2Fkeys&lang=en&utm_medium=email",
    );
    const url = new URL(destination!, "https://nyxid.example");
    expect(url.origin).toBe("https://nyxid.example");
    expect(url.pathname).toBe(target);
    expect(url.searchParams.get("code")).toBe("FRAGMENT-INVITE");
    expect(url.searchParams.get("return_to")).toBe(returnTo);
    expect(url.searchParams.get("lang")).toBe("zh");
    expect(url.searchParams.get("utm_source")).toBe("fragment");
    expect(url.searchParams.get("utm_medium")).toBe("email");
  });

  it("keeps the full value after the first query delimiter", () => {
    const destination = legacyDestination(
      "#/login?return_to=/keys?tab=active",
      "?code=OUTER",
    );
    const url = new URL(destination!, "https://nyxid.example");
    expect(url.pathname).toBe("/login");
    expect(url.searchParams.get("return_to")).toBe("/keys?tab=active");
    expect(url.searchParams.get("code")).toBe("OUTER");
  });

  it.each(["/start", "/done"])(
    "drops mock %s fragment state without losing the outer auth context",
    (path) => {
      expect(
        legacyDestination(
          `#${path}?t=restaurant-manager&code=MOCK&return_to=https%3A%2F%2Fexample.org`,
          "?code=REAL&return_to=%2Fkeys&lang=zh&utm_source=campaign",
        ),
      ).toBe(
        "/solutions/restaurant-manager?code=REAL&return_to=%2Fkeys&lang=zh&utm_source=campaign",
      );
    },
  );
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
  ).toBe("/solutions/restaurant-manager?code=INVITE");
  expect(legacyDestination("#/login", "?return_to=%2Fkeys")).toBe(
    "/login?return_to=%2Fkeys",
  );
});
