import { describe, it, expect } from "vitest";
import {
  createRegistrationTokenSchema,
  createBindingSchema,
  transferNodeSchema,
  nodePendingCredentialInjectionMethodSchema,
  pushNodeCredentialSchema,
  pushNodeCredentialFanOutSchema,
  integrityVerificationSchema,
  pendingCredentialCiphertextRequestSchema,
  fanOutCiphertextsSchema,
  MAX_FAN_OUT_CIPHERTEXT_TOTAL_SIZE,
  acceptNodeCredentialSecretSchema,
  MAX_REMOTE_CREDENTIAL_PLAINTEXT_SIZE,
} from "./nodes";
import { encodeBase64UrlNoPad, MAX_CIPHERTEXT_SIZE } from "@/lib/crypto";

function b64(len: number): string {
  return encodeBase64UrlNoPad(new Uint8Array(len));
}

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

  it("defaults remote_crypto to true", () => {
    const result = pushNodeCredentialSchema.safeParse(base);
    expect(result.success).toBe(true);
    if (result.success) {
      expect(result.data.remote_crypto).toBe(true);
    }
  });

  it("rejects remote_crypto false and has no plaintext secret fields", () => {
    expect(
      pushNodeCredentialSchema.safeParse({ ...base, remote_crypto: false })
        .success,
    ).toBe(false);

    const shape = pushNodeCredentialSchema.shape;
    for (const field of ["secret", "credential", "token", "value"]) {
      expect(shape).not.toHaveProperty(field);
    }
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

describe("acceptNodeCredentialSecretSchema", () => {
  it("accepts local non-empty secret bytes within the maximum size", () => {
    expect(
      acceptNodeCredentialSecretSchema.safeParse(new Uint8Array([1])).success,
    ).toBe(true);
  });

  it("rejects empty and oversized local secret bytes", () => {
    expect(
      acceptNodeCredentialSecretSchema.safeParse(new Uint8Array()).success,
    ).toBe(false);
    expect(
      acceptNodeCredentialSecretSchema.safeParse(
        new Uint8Array(MAX_REMOTE_CREDENTIAL_PLAINTEXT_SIZE + 1),
      ).success,
    ).toBe(false);
  });
});

describe("integrityVerificationSchema", () => {
  const adminVerified = {
    mode: "admin_verified" as const,
    fingerprint_sha384_hex: "a".repeat(96),
    verified_at: "2026-06-05T00:00:00.000Z",
    manifest_url_configured: true,
  };

  const orgPolicyOptOut = {
    mode: "org_policy_opt_out" as const,
    fingerprint_sha384_hex: null,
    verified_at: null,
    manifest_url_configured: false,
  };

  it("accepts admin verification metadata", () => {
    expect(integrityVerificationSchema.safeParse(adminVerified).success).toBe(
      true,
    );
  });

  it("accepts org policy opt-out metadata", () => {
    expect(integrityVerificationSchema.safeParse(orgPolicyOptOut).success).toBe(
      true,
    );
  });

  it("rejects bad modes and missing required fields", () => {
    expect(
      integrityVerificationSchema.safeParse({
        ...adminVerified,
        mode: "manual",
      }).success,
    ).toBe(false);
    expect(
      integrityVerificationSchema.safeParse({
        ...adminVerified,
        fingerprint_sha384_hex: undefined,
      }).success,
    ).toBe(false);
    expect(
      integrityVerificationSchema.safeParse({
        ...adminVerified,
        verified_at: undefined,
      }).success,
    ).toBe(false);
    expect(
      integrityVerificationSchema.safeParse({
        ...adminVerified,
        manifest_url_configured: false,
      }).success,
    ).toBe(false);
  });

  it("rejects oversized or invalid fingerprints", () => {
    expect(
      integrityVerificationSchema.safeParse({
        ...adminVerified,
        fingerprint_sha384_hex: "a".repeat(97),
      }).success,
    ).toBe(false);
    expect(
      integrityVerificationSchema.safeParse({
        ...adminVerified,
        fingerprint_sha384_hex: "A".repeat(96),
      }).success,
    ).toBe(false);
  });

  it("rejects org opt-out metadata with admin verification fields", () => {
    expect(
      integrityVerificationSchema.safeParse({
        ...orgPolicyOptOut,
        fingerprint_sha384_hex: "a".repeat(96),
      }).success,
    ).toBe(false);
    expect(
      integrityVerificationSchema.safeParse({
        ...orgPolicyOptOut,
        verified_at: "2026-06-05T00:00:00.000Z",
      }).success,
    ).toBe(false);
  });
});

describe("pendingCredentialCiphertextRequestSchema", () => {
  const envelope = {
    version: "v1" as const,
    admin_pubkey: b64(32),
    nonce: b64(24),
    ciphertext: b64(32),
  };

  it("accepts ciphertext requests without integrity verification", () => {
    expect(
      pendingCredentialCiphertextRequestSchema.safeParse(envelope).success,
    ).toBe(true);
  });

  it("accepts ciphertext requests with integrity verification", () => {
    expect(
      pendingCredentialCiphertextRequestSchema.safeParse({
        ...envelope,
        integrity_verification: {
          mode: "admin_verified",
          fingerprint_sha384_hex: "a".repeat(96),
          verified_at: "2026-06-05T00:00:00.000Z",
          manifest_url_configured: true,
        },
      }).success,
    ).toBe(true);
  });
});

describe("fanOutCiphertextsSchema", () => {
  const item = {
    node_id: "node-1",
    generation: 0,
    version: "v1" as const,
    admin_pubkey: b64(32),
    nonce: b64(24),
    ciphertext: b64(32),
  };

  it("accepts fan-out envelope array shape", () => {
    expect(
      fanOutCiphertextsSchema.safeParse({
        fan_out_revision: 1,
        items: [item],
      }).success,
    ).toBe(true);
  });

  it("accepts fan-out integrity verification metadata", () => {
    expect(
      fanOutCiphertextsSchema.safeParse({
        fan_out_revision: 1,
        items: [item],
        integrity_verification: {
          mode: "org_policy_opt_out",
          fingerprint_sha384_hex: null,
          verified_at: null,
          manifest_url_configured: false,
        },
      }).success,
    ).toBe(true);
  });

  it("rejects per-element ciphertexts over the cap", () => {
    expect(
      fanOutCiphertextsSchema.safeParse({
        fan_out_revision: 1,
        items: [{ ...item, ciphertext: b64(MAX_CIPHERTEXT_SIZE + 1) }],
      }).success,
    ).toBe(false);
  });

  it("rejects aggregate ciphertext bytes over the cap", () => {
    const full = Array.from({ length: 10 }, (_, index) => ({
      ...item,
      node_id: `node-${String(index)}`,
      ciphertext: b64(MAX_CIPHERTEXT_SIZE),
    }));
    expect(MAX_FAN_OUT_CIPHERTEXT_TOTAL_SIZE).toBe(
      10 * MAX_CIPHERTEXT_SIZE,
    );
    expect(
      fanOutCiphertextsSchema.safeParse({
        fan_out_revision: 1,
        items: [...full, { ...item, node_id: "overflow", ciphertext: b64(1) }],
      }).success,
    ).toBe(false);
  });

  it("push and ciphertext schemas have no plaintext-like fields", () => {
    const pushShape = pushNodeCredentialFanOutSchema.shape;
    const ciphertextShape = fanOutCiphertextsSchema.shape;
    for (const field of ["secret", "credential", "token", "value", "plaintext"]) {
      expect(pushShape).not.toHaveProperty(field);
      expect(ciphertextShape).not.toHaveProperty(field);
    }
  });
});
