import { describe, expect, it } from "vitest";
import { oauthConsentServiceAccessSchema } from "./oauth-consent";

describe("oauthConsentServiceAccessSchema", () => {
  it("clears selected services when all services are allowed", () => {
    const result = oauthConsentServiceAccessSchema.parse({
      allow_all_services: true,
      allowed_service_ids: ["svc-1"],
    });

    expect(result).toEqual({
      allow_all_services: true,
      allowed_service_ids: [],
    });
  });

  it("deduplicates service ids when access is scoped", () => {
    const result = oauthConsentServiceAccessSchema.parse({
      allow_all_services: false,
      allowed_service_ids: ["svc-1", "svc-1", "svc-2"],
    });

    expect(result).toEqual({
      allow_all_services: false,
      allowed_service_ids: ["svc-1", "svc-2"],
    });
  });

  it("rejects blank selected service ids", () => {
    expect(
      oauthConsentServiceAccessSchema.safeParse({
        allow_all_services: false,
        allowed_service_ids: [""],
      }).success,
    ).toBe(false);
  });
});
