import { describe, it, expect } from "vitest";
import type { DownstreamService } from "@/types/api";
import {
  AUTH_TYPE_LABELS,
  SERVICE_CATEGORY_LABELS,
  getAuthTypeLabel,
  isOidcService,
  isConnectable,
  isProvider,
  getCredentialInputType,
} from "./constants";

function makeService(
  overrides: Partial<DownstreamService> = {},
): DownstreamService {
  return {
    id: "svc-1",
    name: "Test Service",
    slug: "test-service",
    description: null,
    base_url: "https://api.example.com",
    auth_method: "api_key",
    auth_type: null,
    auth_key_name: "Authorization",
    is_active: true,
    oauth_client_id: null,
    api_spec_url: null,
    service_category: "connection",
    requires_user_credential: true,
    created_by: "user-1",
    created_at: "2024-01-01T00:00:00Z",
    updated_at: "2024-01-01T00:00:00Z",
    ...overrides,
  };
}

describe("AUTH_TYPE_LABELS", () => {
  it("maps api_key to 'API Key'", () => {
    expect(AUTH_TYPE_LABELS["api_key"]).toBe("API Key");
  });

  it("maps oauth2 to 'OAuth 2.0'", () => {
    expect(AUTH_TYPE_LABELS["oauth2"]).toBe("OAuth 2.0");
  });

  it("maps oidc to 'OIDC / SSO'", () => {
    expect(AUTH_TYPE_LABELS["oidc"]).toBe("OIDC / SSO");
  });
});

describe("SERVICE_CATEGORY_LABELS", () => {
  it("maps provider to 'SSO Provider'", () => {
    expect(SERVICE_CATEGORY_LABELS["provider"]).toBe("SSO Provider");
  });

  it("maps connection to 'External Service'", () => {
    expect(SERVICE_CATEGORY_LABELS["connection"]).toBe("External Service");
  });
});

describe("getAuthTypeLabel", () => {
  it("returns label from auth_type when present", () => {
    const svc = makeService({ auth_type: "oauth2" });
    expect(getAuthTypeLabel(svc)).toBe("OAuth 2.0");
  });

  it("falls back to auth_method when auth_type is null", () => {
    const svc = makeService({ auth_type: null, auth_method: "bearer" });
    expect(getAuthTypeLabel(svc)).toBe("Bearer Token");
  });

  it("returns raw key for unknown type", () => {
    const svc = makeService({
      auth_type: null,
      auth_method: "custom_unknown",
    });
    expect(getAuthTypeLabel(svc)).toBe("custom_unknown");
  });
});

describe("isOidcService", () => {
  it("returns true when auth_method is oidc", () => {
    expect(isOidcService(makeService({ auth_method: "oidc" }))).toBe(true);
  });

  it("returns true when auth_type is oidc", () => {
    expect(isOidcService(makeService({ auth_type: "oidc" }))).toBe(true);
  });

  it("returns true when oauth_client_id is set", () => {
    expect(
      isOidcService(makeService({ oauth_client_id: "client-123" })),
    ).toBe(true);
  });

  it("returns false when none of the conditions match", () => {
    expect(
      isOidcService(
        makeService({
          auth_method: "api_key",
          auth_type: null,
          oauth_client_id: null,
        }),
      ),
    ).toBe(false);
  });
});

describe("isConnectable", () => {
  it("returns true for connection category", () => {
    expect(isConnectable(makeService({ service_category: "connection" }))).toBe(
      true,
    );
  });

  it("returns true for internal category", () => {
    expect(isConnectable(makeService({ service_category: "internal" }))).toBe(
      true,
    );
  });

  it("returns false for provider category", () => {
    expect(isConnectable(makeService({ service_category: "provider" }))).toBe(
      false,
    );
  });
});

describe("isProvider", () => {
  it("returns true for provider category", () => {
    expect(isProvider(makeService({ service_category: "provider" }))).toBe(
      true,
    );
  });

  it("returns false for connection category", () => {
    expect(isProvider(makeService({ service_category: "connection" }))).toBe(
      false,
    );
  });
});

describe("getCredentialInputType", () => {
  it("returns none when requires_user_credential is false", () => {
    const svc = makeService({ requires_user_credential: false });
    expect(getCredentialInputType(svc)).toEqual({
      type: "none",
      label: "",
      placeholder: "",
    });
  });

  it("returns api_key type for api_key auth", () => {
    const svc = makeService({
      auth_type: "api_key",
      requires_user_credential: true,
    });
    const result = getCredentialInputType(svc);
    expect(result.type).toBe("api_key");
    expect(result.label).toBe("API Key");
  });

  it("returns bearer type for bearer auth", () => {
    const svc = makeService({
      auth_type: "bearer",
      requires_user_credential: true,
    });
    const result = getCredentialInputType(svc);
    expect(result.type).toBe("bearer");
    expect(result.label).toBe("Bearer Token");
  });

  it("returns basic type for basic auth", () => {
    const svc = makeService({
      auth_type: "basic",
      requires_user_credential: true,
    });
    const result = getCredentialInputType(svc);
    expect(result.type).toBe("basic");
  });

  it("returns bearer type for oauth2 auth", () => {
    const svc = makeService({
      auth_type: "oauth2",
      requires_user_credential: true,
    });
    const result = getCredentialInputType(svc);
    expect(result.type).toBe("bearer");
    expect(result.label).toBe("Access Token");
  });

  it("falls back to api_key for unknown auth type", () => {
    const svc = makeService({
      auth_type: "unknown",
      auth_method: "unknown",
      requires_user_credential: true,
    });
    const result = getCredentialInputType(svc);
    expect(result.type).toBe("api_key");
    expect(result.label).toBe("Credential");
  });

  it("uses auth_method when auth_type is null", () => {
    const svc = makeService({
      auth_type: null,
      auth_method: "api_key",
      requires_user_credential: true,
    });
    const result = getCredentialInputType(svc);
    expect(result.type).toBe("api_key");
  });
});
