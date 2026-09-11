import { describe, expect, it } from "vitest";
import { isPublicPath } from "./public-paths";

describe("public route policy", () => {
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
