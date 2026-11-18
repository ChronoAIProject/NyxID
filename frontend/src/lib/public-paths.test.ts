import { describe, expect, it } from "vitest";
import { isPublicPath } from "./public-paths";

describe("public route policy", () => {
  it("allows the exact device entry without opening code paths or unrelated routes", () => {
    expect(isPublicPath("/login/device")).toBe(true);
    expect(isPublicPath("/login/device/ABCD-EFGH")).toBe(false);
    expect(isPublicPath("/login/device-admin")).toBe(false);
    expect(isPublicPath("/login/device/ABCD-EFGH/admin")).toBe(false);
  });
  it("allows the exact onboarding entry before auth without broadening the route prefix", () => {
    expect(isPublicPath("/nyxbot/onboarding")).toBe(true);
    expect(isPublicPath("/nyxbot/onboarding/admin")).toBe(false);
  });
  it("renders both OAuth popup routes before auth resolution", () => {
    expect(isPublicPath("/oauth-complete")).toBe(true);
    expect(isPublicPath("/oauth-launching")).toBe(true);
  });

  it("does not broaden the OAuth exception to backend IdP paths", () => {
    expect(isPublicPath("/oauth/authorize")).toBe(false);
    expect(isPublicPath("/oauth/token")).toBe(false);
  });
});

it("opens only the website routes and one solution segment, including a trailing slash", () => {
  for (const path of [
    "/nyxbot",
    "/solutions",
    "/how-it-works",
    "/security",
    "/agents",
    "/integrations",
    "/pricing",
    "/contact",
    "/solutions/restaurant-manager",
  ]) {
    expect(isPublicPath(path)).toBe(true);
    expect(isPublicPath(`${path}/`)).toBe(true);
  }
  for (const path of [
    "/nyxbot/admin",
    "/security/admin",
    "/solutions/one/admin",
    "/agents/keys",
    "/integrations/admin",
    "/api/v1/services",
    "/dashboard",
    "/assistant",
  ]) {
    expect(isPublicPath(path)).toBe(false);
  }
});
