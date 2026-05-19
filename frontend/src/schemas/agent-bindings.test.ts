import { describe, it, expect } from "vitest";
import {
  createBindingSchema,
  updateRateLimitSchema,
  PLATFORM_OPTIONS,
} from "./agent-bindings";

describe("createBindingSchema", () => {
  it("accepts valid binding data", () => {
    const result = createBindingSchema.safeParse({
      user_service_id: "svc-123",
      user_api_key_id: "key-456",
    });
    expect(result.success).toBe(true);
  });

  it("rejects empty service id", () => {
    const result = createBindingSchema.safeParse({
      user_service_id: "",
      user_api_key_id: "key-456",
    });
    expect(result.success).toBe(false);
  });

  it("rejects empty credential id", () => {
    const result = createBindingSchema.safeParse({
      user_service_id: "svc-123",
      user_api_key_id: "",
    });
    expect(result.success).toBe(false);
  });

  it("rejects missing fields", () => {
    const result = createBindingSchema.safeParse({});
    expect(result.success).toBe(false);
  });
});

describe("updateRateLimitSchema", () => {
  it("accepts valid rate limit data", () => {
    const result = updateRateLimitSchema.safeParse({
      rate_limit_per_second: 10,
      rate_limit_burst: 30,
    });
    expect(result.success).toBe(true);
  });

  it("accepts null values (use defaults)", () => {
    const result = updateRateLimitSchema.safeParse({
      rate_limit_per_second: null,
      rate_limit_burst: null,
    });
    expect(result.success).toBe(true);
  });

  it("rejects zero rate limit per second", () => {
    const result = updateRateLimitSchema.safeParse({
      rate_limit_per_second: 0,
      rate_limit_burst: 30,
    });
    expect(result.success).toBe(false);
  });

  it("rejects negative burst", () => {
    const result = updateRateLimitSchema.safeParse({
      rate_limit_per_second: 10,
      rate_limit_burst: -1,
    });
    expect(result.success).toBe(false);
  });

  it("rejects non-integer values", () => {
    const result = updateRateLimitSchema.safeParse({
      rate_limit_per_second: 1.5,
      rate_limit_burst: 30,
    });
    expect(result.success).toBe(false);
  });

  it("rejects rate limit over 10000", () => {
    const result = updateRateLimitSchema.safeParse({
      rate_limit_per_second: 10001,
      rate_limit_burst: 30,
    });
    expect(result.success).toBe(false);
  });

  it("rejects burst over 100000", () => {
    const result = updateRateLimitSchema.safeParse({
      rate_limit_per_second: 10,
      rate_limit_burst: 100001,
    });
    expect(result.success).toBe(false);
  });
});

describe("PLATFORM_OPTIONS", () => {
  it("contains expected platforms", () => {
    expect(PLATFORM_OPTIONS).toContain("claude-code");
    expect(PLATFORM_OPTIONS).toContain("cursor");
    expect(PLATFORM_OPTIONS).toContain("codex");
    expect(PLATFORM_OPTIONS).toContain("openclaw");
    expect(PLATFORM_OPTIONS).toContain("generic");
  });

  it("has 5 options", () => {
    expect(PLATFORM_OPTIONS).toHaveLength(5);
  });
});
