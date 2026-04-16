import { describe, it, expect } from "vitest";
import { createApiKeySchema, API_KEY_SCOPES } from "./api-keys";

describe("createApiKeySchema", () => {
  it("accepts valid API key data", () => {
    const result = createApiKeySchema.safeParse({
      name: "My API Key",
      scopes: ["read"],
    });
    expect(result.success).toBe(true);
  });

  it("accepts data with nullable expires_at", () => {
    const result = createApiKeySchema.safeParse({
      name: "Test Key",
      scopes: ["read", "write"],
      expires_at: null,
    });
    expect(result.success).toBe(true);
  });

  it("accepts data with future expires_at", () => {
    const future = new Date(Date.now() + 30 * 24 * 60 * 60 * 1000).toISOString();
    const result = createApiKeySchema.safeParse({
      name: "Test Key",
      scopes: ["admin"],
      expires_at: future,
    });
    expect(result.success).toBe(true);
  });

  it("accepts a future date-only expires_at (treated as 23:59:59 UTC)", () => {
    const future = new Date(Date.now() + 30 * 24 * 60 * 60 * 1000)
      .toISOString()
      .slice(0, 10);
    const result = createApiKeySchema.safeParse({
      name: "Test Key",
      scopes: ["admin"],
      expires_at: future,
    });
    expect(result.success).toBe(true);
  });

  it("rejects past expires_at (RFC 3339)", () => {
    const result = createApiKeySchema.safeParse({
      name: "Test Key",
      scopes: ["admin"],
      expires_at: "2020-01-01T00:00:00Z",
    });
    expect(result.success).toBe(false);
  });

  it("rejects past expires_at (date-only)", () => {
    const result = createApiKeySchema.safeParse({
      name: "Test Key",
      scopes: ["admin"],
      expires_at: "2020-01-01",
    });
    expect(result.success).toBe(false);
  });

  it("rejects malformed expires_at", () => {
    const result = createApiKeySchema.safeParse({
      name: "Test Key",
      scopes: ["admin"],
      expires_at: "not-a-date",
    });
    expect(result.success).toBe(false);
  });

  it("accepts proxy scope for service access", () => {
    const result = createApiKeySchema.safeParse({
      name: "Proxy Key",
      scopes: ["read", "proxy"],
    });
    expect(result.success).toBe(true);
  });

  it("rejects empty name", () => {
    const result = createApiKeySchema.safeParse({
      name: "",
      scopes: ["read"],
    });
    expect(result.success).toBe(false);
  });

  it("rejects name over 64 characters", () => {
    const result = createApiKeySchema.safeParse({
      name: "a".repeat(65),
      scopes: ["read"],
    });
    expect(result.success).toBe(false);
  });

  it("rejects empty scopes array", () => {
    const result = createApiKeySchema.safeParse({
      name: "Test Key",
      scopes: [],
    });
    expect(result.success).toBe(false);
  });

  it("rejects invalid scope values", () => {
    const result = createApiKeySchema.safeParse({
      name: "Test Key",
      scopes: ["invalid_scope"],
    });
    expect(result.success).toBe(false);
  });

  it("rejects old frontend scope format", () => {
    const result = createApiKeySchema.safeParse({
      name: "Test Key",
      scopes: ["read:profile"],
    });
    expect(result.success).toBe(false);
  });

  it("accepts all valid scopes", () => {
    const result = createApiKeySchema.safeParse({
      name: "Full Access Key",
      scopes: [...API_KEY_SCOPES],
    });
    expect(result.success).toBe(true);
  });
});

describe("API_KEY_SCOPES", () => {
  it("contains expected scopes", () => {
    expect(API_KEY_SCOPES).toContain("read");
    expect(API_KEY_SCOPES).toContain("write");
    expect(API_KEY_SCOPES).toContain("proxy");
    expect(API_KEY_SCOPES).toContain("admin");
  });

  it("has 9 scopes", () => {
    expect(API_KEY_SCOPES).toHaveLength(9);
  });
});
