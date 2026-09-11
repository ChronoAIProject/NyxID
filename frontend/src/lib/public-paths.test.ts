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

  it("lets connect pages preserve their request through login", () => {
    expect(isPublicPath("/connect/nyx_clk_example")).toBe(true);
    expect(
      isPublicPath("/connect/return/11111111-1111-4111-8111-111111111111"),
    ).toBe(true);
    expect(isPublicPath("/connect/nyx_clk_example/")).toBe(true);
  });

  it("limits the connect exception to its two page shapes", () => {
    expect(isPublicPath("/connect")).toBe(false);
    expect(isPublicPath("/connect/")).toBe(false);
    expect(isPublicPath("/connect/return/id/extra")).toBe(false);
    expect(isPublicPath("/connect/token/extra")).toBe(false);
    expect(isPublicPath("/keys")).toBe(false);
  });
});
