import { describe, it, expect } from "vitest";
import {
  createServiceSchema,
  updateServiceSchema,
  redirectUriSchema,
  AUTH_TYPES,
  SERVICE_CATEGORIES,
  IDENTITY_PROPAGATION_MODES,
} from "./services";

describe("constants", () => {
  it("AUTH_TYPES contains expected values", () => {
    expect(AUTH_TYPES).toEqual(["api_key", "oauth2", "basic", "bearer", "oidc"]);
  });

  it("SERVICE_CATEGORIES contains expected values", () => {
    expect(SERVICE_CATEGORIES).toEqual(["provider", "connection", "internal"]);
  });

  it("IDENTITY_PROPAGATION_MODES contains expected values", () => {
    expect(IDENTITY_PROPAGATION_MODES).toEqual([
      "none",
      "headers",
      "jwt",
      "both",
    ]);
  });
});

describe("createServiceSchema", () => {
  const validData = {
    name: "My Service",
    base_url: "https://api.example.com",
    auth_type: "api_key" as const,
  };

  it("accepts valid service data", () => {
    const result = createServiceSchema.safeParse(validData);
    expect(result.success).toBe(true);
  });

  it("accepts data with optional fields", () => {
    const result = createServiceSchema.safeParse({
      ...validData,
      description: "A test service",
      service_category: "connection" as const,
    });
    expect(result.success).toBe(true);
  });

  it("rejects empty name", () => {
    const result = createServiceSchema.safeParse({
      ...validData,
      name: "",
    });
    expect(result.success).toBe(false);
  });

  it("rejects name over 200 characters", () => {
    const result = createServiceSchema.safeParse({
      ...validData,
      name: "a".repeat(201),
    });
    expect(result.success).toBe(false);
  });

  it("rejects invalid base URL", () => {
    const result = createServiceSchema.safeParse({
      ...validData,
      base_url: "not-a-url",
    });
    expect(result.success).toBe(false);
  });

  it("rejects empty base URL", () => {
    const result = createServiceSchema.safeParse({
      ...validData,
      base_url: "",
    });
    expect(result.success).toBe(false);
  });

  it("rejects invalid auth_type", () => {
    const result = createServiceSchema.safeParse({
      ...validData,
      auth_type: "invalid",
    });
    expect(result.success).toBe(false);
  });

  it("accepts all valid auth types", () => {
    for (const authType of AUTH_TYPES) {
      const result = createServiceSchema.safeParse({
        ...validData,
        auth_type: authType,
      });
      expect(result.success).toBe(true);
    }
  });

  it("rejects description over 500 characters", () => {
    const result = createServiceSchema.safeParse({
      ...validData,
      description: "a".repeat(501),
    });
    expect(result.success).toBe(false);
  });
});

describe("updateServiceSchema", () => {
  const validData = {
    name: "Updated Service",
    base_url: "https://api.example.com",
  };

  it("accepts valid update data", () => {
    const result = updateServiceSchema.safeParse(validData);
    expect(result.success).toBe(true);
  });

  it("accepts update with identity propagation fields", () => {
    const result = updateServiceSchema.safeParse({
      ...validData,
      identity_propagation_mode: "headers" as const,
      identity_include_user_id: true,
      identity_include_email: false,
      identity_include_name: true,
      identity_jwt_audience: "https://audience.example.com",
    });
    expect(result.success).toBe(true);
  });

  it("accepts empty string for optional URL fields", () => {
    const result = updateServiceSchema.safeParse({
      ...validData,
      api_spec_url: "",
      description: "",
    });
    expect(result.success).toBe(true);
  });

  it("rejects invalid api_spec_url", () => {
    const result = updateServiceSchema.safeParse({
      ...validData,
      api_spec_url: "not-a-url",
    });
    expect(result.success).toBe(false);
  });
});

describe("redirectUriSchema", () => {
  it("accepts https URL", () => {
    const result = redirectUriSchema.safeParse("https://example.com/callback");
    expect(result.success).toBe(true);
  });

  it("accepts http URL", () => {
    const result = redirectUriSchema.safeParse(
      "http://localhost:3000/callback",
    );
    expect(result.success).toBe(true);
  });

  it("rejects empty string", () => {
    const result = redirectUriSchema.safeParse("");
    expect(result.success).toBe(false);
  });

  it("rejects non-URL string", () => {
    const result = redirectUriSchema.safeParse("not a url");
    expect(result.success).toBe(false);
  });

  it("rejects javascript: scheme", () => {
    const result = redirectUriSchema.safeParse("javascript:alert(1)");
    expect(result.success).toBe(false);
  });

  it("rejects ftp: scheme", () => {
    const result = redirectUriSchema.safeParse("ftp://files.example.com");
    expect(result.success).toBe(false);
  });
});
