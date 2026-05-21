import { describe, it, expect } from "vitest";
import {
  createRegistrationTokenSchema,
  createBindingSchema,
  transferNodeSchema,
  nodePendingCredentialInjectionMethodSchema,
  pushNodeCredentialSchema,
} from "./nodes";

describe("createRegistrationTokenSchema", () => {
  it("accepts a valid lowercase-hyphen name with no owner", () => {
    expect(
      createRegistrationTokenSchema.safeParse({ name: "edge-node-1" }).success,
    ).toBe(true);
  });

  it("rejects names that start or end with a hyphen", () => {
    expect(createRegistrationTokenSchema.safeParse({ name: "-node" }).success).toBe(false);
    expect(createRegistrationTokenSchema.safeParse({ name: "node-" }).success).toBe(false);
  });

  it("rejects uppercase and other disallowed characters", () => {
    expect(createRegistrationTokenSchema.safeParse({ name: "Node" }).success).toBe(false);
    expect(createRegistrationTokenSchema.safeParse({ name: "node_1" }).success).toBe(false);
  });

  it("rejects empty and over-64-character names", () => {
    expect(createRegistrationTokenSchema.safeParse({ name: "" }).success).toBe(false);
    expect(
      createRegistrationTokenSchema.safeParse({ name: "a".repeat(65) }).success,
    ).toBe(false);
  });

  it("allows owner_user_id to be a string, null, or omitted", () => {
    expect(
      createRegistrationTokenSchema.safeParse({ name: "n", owner_user_id: "u-1" }).success,
    ).toBe(true);
    expect(
      createRegistrationTokenSchema.safeParse({ name: "n", owner_user_id: null }).success,
    ).toBe(true);
    expect(createRegistrationTokenSchema.safeParse({ name: "n" }).success).toBe(true);
  });
});

describe("createBindingSchema / transferNodeSchema", () => {
  it("require their respective id fields", () => {
    expect(createBindingSchema.safeParse({ service_id: "svc" }).success).toBe(true);
    expect(createBindingSchema.safeParse({ service_id: "" }).success).toBe(false);
    expect(
      transferNodeSchema.safeParse({ new_owner_user_id: "owner" }).success,
    ).toBe(true);
    expect(transferNodeSchema.safeParse({ new_owner_user_id: "" }).success).toBe(false);
  });
});

describe("nodePendingCredentialInjectionMethodSchema", () => {
  it("accepts only the three known methods", () => {
    for (const m of ["header", "query-param", "path-prefix"]) {
      expect(nodePendingCredentialInjectionMethodSchema.safeParse(m).success).toBe(true);
    }
    expect(nodePendingCredentialInjectionMethodSchema.safeParse("body").success).toBe(false);
  });
});

describe("pushNodeCredentialSchema", () => {
  const base = {
    service_slug: "openai",
    injection_method: "header" as const,
    field_name: "Authorization",
  };

  it("accepts a minimal valid credential push", () => {
    expect(pushNodeCredentialSchema.safeParse(base).success).toBe(true);
  });

  it("rejects an invalid service slug", () => {
    expect(
      pushNodeCredentialSchema.safeParse({ ...base, service_slug: "Open AI" }).success,
    ).toBe(false);
  });

  it("rejects control characters in field_name", () => {
    expect(
      pushNodeCredentialSchema.safeParse({ ...base, field_name: "X-Api\x07Key" }).success,
    ).toBe(false);
  });

  it("treats blank target_url / label as undefined rather than invalid", () => {
    const result = pushNodeCredentialSchema.safeParse({
      ...base,
      target_url: "   ",
      label: "",
    });
    expect(result.success).toBe(true);
    if (result.success) {
      expect(result.data.target_url).toBeUndefined();
      expect(result.data.label).toBeUndefined();
    }
  });

  it("validates a non-blank target_url as a URL", () => {
    expect(
      pushNodeCredentialSchema.safeParse({ ...base, target_url: "not-a-url" }).success,
    ).toBe(false);
    expect(
      pushNodeCredentialSchema.safeParse({ ...base, target_url: "https://api.openai.com" })
        .success,
    ).toBe(true);
  });
});
