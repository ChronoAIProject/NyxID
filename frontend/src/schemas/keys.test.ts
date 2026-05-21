import { describe, it, expect } from "vitest";
import {
  userServiceResponseSchema,
  updateUserServiceRequestSchema,
  userServiceListResponseSchema,
} from "./keys";

const validResponse = {
  id: "us-1",
  slug: "openai",
  endpoint_id: "ep-1",
  api_key_id: null,
  auth_method: "bearer",
  auth_key_name: "Authorization",
  catalog_service_id: null,
  node_id: null,
  node_priority: 0,
  is_active: true,
  identity_propagation_mode: "none",
  identity_include_user_id: false,
  identity_include_email: false,
  identity_include_name: false,
  identity_jwt_audience: null,
  forward_access_token: false,
  inject_delegation_token: false,
  delegation_token_scope: "",
  custom_user_agent: null,
  ws_frame_injections: [],
  created_at: "2026-05-01T00:00:00Z",
  updated_at: "2026-05-01T00:00:00Z",
  credential_source: { type: "personal" as const },
};

describe("userServiceResponseSchema", () => {
  it("accepts a valid personal-source user service", () => {
    expect(userServiceResponseSchema.safeParse(validResponse).success).toBe(true);
  });

  it("accepts an org-sourced credential_source", () => {
    const result = userServiceResponseSchema.safeParse({
      ...validResponse,
      credential_source: {
        type: "org",
        org_id: "org-1",
        org_name: "Acme",
        role: "admin",
        allowed: true,
      },
    });
    expect(result.success).toBe(true);
  });

  it("is permissive about unknown backend fields (validation does not fail, keys are stripped)", () => {
    const result = userServiceResponseSchema.safeParse({
      ...validResponse,
      some_future_field: "ignored",
    });
    // A newer backend can add fields without breaking the client: parse
    // succeeds. Zod's default object behaviour strips the unknown key.
    expect(result.success).toBe(true);
    if (result.success) {
      expect((result.data as Record<string, unknown>).some_future_field).toBeUndefined();
      expect(result.data.slug).toBe("openai");
    }
  });

  it("rejects when a required field is missing or mistyped", () => {
    const withoutSlug: Partial<typeof validResponse> = { ...validResponse };
    delete withoutSlug.slug;
    expect(userServiceResponseSchema.safeParse(withoutSlug).success).toBe(false);
    expect(
      userServiceResponseSchema.safeParse({ ...validResponse, node_priority: 1.5 }).success,
    ).toBe(false);
  });
});

describe("updateUserServiceRequestSchema", () => {
  it("accepts an empty partial update", () => {
    expect(updateUserServiceRequestSchema.safeParse({}).success).toBe(true);
  });

  it("supports the default_request_headers tri-state (undefined / null / array)", () => {
    expect(updateUserServiceRequestSchema.safeParse({}).success).toBe(true);
    expect(
      updateUserServiceRequestSchema.safeParse({ default_request_headers: null }).success,
    ).toBe(true);
    expect(
      updateUserServiceRequestSchema.safeParse({
        default_request_headers: [
          { name: "X-Trace-Id", value: "abc", overridable: true, sensitive: false },
        ],
      }).success,
    ).toBe(true);
  });

  it("rejects a non-integer node_priority", () => {
    expect(
      updateUserServiceRequestSchema.safeParse({ node_priority: 2.5 }).success,
    ).toBe(false);
  });
});

describe("userServiceListResponseSchema", () => {
  it("wraps the response array under `services`", () => {
    expect(
      userServiceListResponseSchema.safeParse({ services: [validResponse] }).success,
    ).toBe(true);
    expect(userServiceListResponseSchema.safeParse({ services: "nope" }).success).toBe(false);
  });
});
