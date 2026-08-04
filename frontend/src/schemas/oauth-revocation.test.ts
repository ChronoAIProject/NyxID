import { describe, expect, it } from "vitest";
import {
  grantCascadeErrorResponseSchema,
  parseGrantCascadeDetails,
} from "./oauth-revocation";

const payload = {
  error: "grant_cascade_confirmation_required",
  error_code: 11500,
  message: "Confirmation required",
  details: {
    provider_slug: "github",
    provider_name: "GitHub",
    revokes_grant: true,
    siblings: [
      {
        user_service_id: "service-2",
        name: "GitHub Issues",
        slug: "github-issues",
      },
    ],
    unaffected_other_app: [],
    token_scope_available: true,
  },
};

describe("grantCascadeErrorResponseSchema", () => {
  it("parses the typed 11500 details payload", () => {
    expect(
      grantCascadeErrorResponseSchema.parse(payload).details.siblings,
    ).toHaveLength(1);
    expect(parseGrantCascadeDetails(payload)?.provider_name).toBe("GitHub");
  });

  it("parses non-service siblings with empty service identifiers", () => {
    const details = grantCascadeErrorResponseSchema.parse({
      ...payload,
      details: {
        ...payload.details,
        siblings: [
          {
            user_service_id: "",
            name: "GitHub provider connection",
            slug: "",
          },
        ],
      },
    }).details;

    expect(details.siblings[0]?.user_service_id).toBe("");
    expect(details.siblings[0]?.slug).toBe("");
  });

  it("rejects another error code or malformed details", () => {
    expect(
      parseGrantCascadeDetails({ ...payload, error_code: 1008 }),
    ).toBeNull();
    expect(
      parseGrantCascadeDetails({
        ...payload,
        details: { ...payload.details, siblings: [{ name: "Missing IDs" }] },
      }),
    ).toBeNull();
  });
});
