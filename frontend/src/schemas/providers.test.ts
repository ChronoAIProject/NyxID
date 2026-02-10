import { describe, it, expect } from "vitest";
import {
  connectApiKeySchema,
  createProviderSchema,
  updateProviderSchema,
  PROVIDER_TYPES,
} from "./providers";

describe("PROVIDER_TYPES", () => {
  it("contains expected types", () => {
    expect(PROVIDER_TYPES).toEqual(["oauth2", "api_key", "device_code"]);
  });
});

describe("connectApiKeySchema", () => {
  it("accepts valid API key", () => {
    const result = connectApiKeySchema.safeParse({
      api_key: "sk-1234567890abcdef",
    });
    expect(result.success).toBe(true);
  });

  it("accepts API key with optional label", () => {
    const result = connectApiKeySchema.safeParse({
      api_key: "sk-1234567890abcdef",
      label: "Production key",
    });
    expect(result.success).toBe(true);
  });

  it("rejects empty API key", () => {
    const result = connectApiKeySchema.safeParse({ api_key: "" });
    expect(result.success).toBe(false);
  });

  it("rejects API key over 8192 characters", () => {
    const result = connectApiKeySchema.safeParse({
      api_key: "a".repeat(8193),
    });
    expect(result.success).toBe(false);
  });

  it("rejects label over 200 characters", () => {
    const result = connectApiKeySchema.safeParse({
      api_key: "sk-valid",
      label: "a".repeat(201),
    });
    expect(result.success).toBe(false);
  });
});

describe("createProviderSchema", () => {
  const baseValid = {
    name: "Test Provider",
    slug: "test-provider",
    provider_type: "api_key" as const,
  };

  it("accepts valid api_key provider", () => {
    const result = createProviderSchema.safeParse(baseValid);
    expect(result.success).toBe(true);
  });

  it("accepts valid oauth2 provider with required fields", () => {
    const result = createProviderSchema.safeParse({
      ...baseValid,
      provider_type: "oauth2",
      authorization_url: "https://auth.example.com/authorize",
      token_url: "https://auth.example.com/token",
      client_id: "my-client-id",
      client_secret: "my-secret",
    });
    expect(result.success).toBe(true);
  });

  it("rejects oauth2 provider without authorization_url", () => {
    const result = createProviderSchema.safeParse({
      ...baseValid,
      provider_type: "oauth2",
      token_url: "https://auth.example.com/token",
      client_id: "my-client-id",
      client_secret: "my-secret",
    });
    expect(result.success).toBe(false);
  });

  it("rejects oauth2 provider without token_url", () => {
    const result = createProviderSchema.safeParse({
      ...baseValid,
      provider_type: "oauth2",
      authorization_url: "https://auth.example.com/authorize",
      client_id: "my-client-id",
      client_secret: "my-secret",
    });
    expect(result.success).toBe(false);
  });

  it("rejects oauth2 provider without client_id", () => {
    const result = createProviderSchema.safeParse({
      ...baseValid,
      provider_type: "oauth2",
      authorization_url: "https://auth.example.com/authorize",
      token_url: "https://auth.example.com/token",
      client_secret: "my-secret",
    });
    expect(result.success).toBe(false);
  });

  it("rejects oauth2 provider without client_secret", () => {
    const result = createProviderSchema.safeParse({
      ...baseValid,
      provider_type: "oauth2",
      authorization_url: "https://auth.example.com/authorize",
      token_url: "https://auth.example.com/token",
      client_id: "my-client-id",
    });
    expect(result.success).toBe(false);
  });

  it("accepts valid device_code provider", () => {
    const result = createProviderSchema.safeParse({
      ...baseValid,
      provider_type: "device_code",
      authorization_url: "https://auth.example.com/authorize",
      token_url: "https://auth.example.com/token",
      client_id: "my-client-id",
      device_code_url: "https://auth.example.com/device/code",
      device_token_url: "https://auth.example.com/device/token",
    });
    expect(result.success).toBe(true);
  });

  it("rejects device_code provider without device_code_url", () => {
    const result = createProviderSchema.safeParse({
      ...baseValid,
      provider_type: "device_code",
      authorization_url: "https://auth.example.com/authorize",
      token_url: "https://auth.example.com/token",
      client_id: "my-client-id",
      device_token_url: "https://auth.example.com/device/token",
    });
    expect(result.success).toBe(false);
  });

  it("rejects device_code provider without device_token_url", () => {
    const result = createProviderSchema.safeParse({
      ...baseValid,
      provider_type: "device_code",
      authorization_url: "https://auth.example.com/authorize",
      token_url: "https://auth.example.com/token",
      client_id: "my-client-id",
      device_code_url: "https://auth.example.com/device/code",
    });
    expect(result.success).toBe(false);
  });

  it("rejects slug with uppercase letters", () => {
    const result = createProviderSchema.safeParse({
      ...baseValid,
      slug: "Test-Provider",
    });
    expect(result.success).toBe(false);
  });

  it("rejects slug with leading hyphen", () => {
    const result = createProviderSchema.safeParse({
      ...baseValid,
      slug: "-test-provider",
    });
    expect(result.success).toBe(false);
  });

  it("rejects slug with trailing hyphen", () => {
    const result = createProviderSchema.safeParse({
      ...baseValid,
      slug: "test-provider-",
    });
    expect(result.success).toBe(false);
  });

  it("rejects slug shorter than 2 characters", () => {
    const result = createProviderSchema.safeParse({
      ...baseValid,
      slug: "a",
    });
    expect(result.success).toBe(false);
  });

  it("rejects name shorter than 2 characters", () => {
    const result = createProviderSchema.safeParse({
      ...baseValid,
      name: "A",
    });
    expect(result.success).toBe(false);
  });

  it("rejects name over 100 characters", () => {
    const result = createProviderSchema.safeParse({
      ...baseValid,
      name: "a".repeat(101),
    });
    expect(result.success).toBe(false);
  });
});

describe("updateProviderSchema", () => {
  const baseValid = {
    name: "Updated Provider",
    slug: "updated-provider",
    provider_type: "api_key" as const,
  };

  it("accepts valid update data", () => {
    const result = updateProviderSchema.safeParse(baseValid);
    expect(result.success).toBe(true);
  });

  it("accepts update with is_active", () => {
    const result = updateProviderSchema.safeParse({
      ...baseValid,
      is_active: false,
    });
    expect(result.success).toBe(true);
  });

  it("rejects oauth2 update without authorization_url", () => {
    const result = updateProviderSchema.safeParse({
      ...baseValid,
      provider_type: "oauth2",
      token_url: "https://auth.example.com/token",
    });
    expect(result.success).toBe(false);
  });

  it("accepts device_code update without device_code_url (optional on update)", () => {
    const result = updateProviderSchema.safeParse({
      ...baseValid,
      provider_type: "device_code",
      authorization_url: "https://auth.example.com/authorize",
      token_url: "https://auth.example.com/token",
    });
    expect(result.success).toBe(true);
  });
});
