import { describe, it, expect } from "vitest";
import {
  approvalRuleSchema,
  setServiceApprovalConfigSchema,
  updateNotificationSettingsSchema,
} from "./approvals";

const valid = {
  telegram_enabled: true,
  push_enabled: false,
  approval_required: true,
  approval_timeout_secs: 60,
  grant_expiry_days: 30,
};

describe("updateNotificationSettingsSchema", () => {
  it("accepts a fully valid payload", () => {
    expect(updateNotificationSettingsSchema.safeParse(valid).success).toBe(true);
  });

  it("enforces the 10–300s approval_timeout_secs bounds inclusively", () => {
    expect(
      updateNotificationSettingsSchema.safeParse({ ...valid, approval_timeout_secs: 10 })
        .success,
    ).toBe(true);
    expect(
      updateNotificationSettingsSchema.safeParse({ ...valid, approval_timeout_secs: 300 })
        .success,
    ).toBe(true);
    expect(
      updateNotificationSettingsSchema.safeParse({ ...valid, approval_timeout_secs: 9 })
        .success,
    ).toBe(false);
    expect(
      updateNotificationSettingsSchema.safeParse({ ...valid, approval_timeout_secs: 301 })
        .success,
    ).toBe(false);
  });

  it("enforces the 1–365 grant_expiry_days bounds inclusively", () => {
    expect(
      updateNotificationSettingsSchema.safeParse({ ...valid, grant_expiry_days: 1 }).success,
    ).toBe(true);
    expect(
      updateNotificationSettingsSchema.safeParse({ ...valid, grant_expiry_days: 365 }).success,
    ).toBe(true);
    expect(
      updateNotificationSettingsSchema.safeParse({ ...valid, grant_expiry_days: 0 }).success,
    ).toBe(false);
    expect(
      updateNotificationSettingsSchema.safeParse({ ...valid, grant_expiry_days: 366 }).success,
    ).toBe(false);
  });

  it("rejects non-integer numeric fields", () => {
    expect(
      updateNotificationSettingsSchema.safeParse({ ...valid, approval_timeout_secs: 60.5 })
        .success,
    ).toBe(false);
    expect(
      updateNotificationSettingsSchema.safeParse({ ...valid, grant_expiry_days: 1.5 }).success,
    ).toBe(false);
  });

  it("requires the boolean flags to be present", () => {
    const withoutFlag: Partial<typeof valid> = { ...valid };
    delete withoutFlag.telegram_enabled;
    expect(updateNotificationSettingsSchema.safeParse(withoutFlag).success).toBe(false);
  });
});

describe("approvalRuleSchema", () => {
  it("accepts a simple verb-only rule", () => {
    const result = approvalRuleSchema.safeParse({
      methods: [],
      resource_pattern: "*",
      verbs: ["write"],
      effect: "require_approval",
      mode: "grant",
    });

    expect(result.success).toBe(true);
  });

  it("rejects unsupported methods and oversized patterns", () => {
    expect(
      approvalRuleSchema.safeParse({
        methods: ["BREW"],
        resource_pattern: "*",
        verbs: ["read"],
        effect: "auto_allow",
        mode: "per_request",
      }).success,
    ).toBe(false);

    expect(
      approvalRuleSchema.safeParse({
        methods: ["GET"],
        resource_pattern: "/".repeat(257),
        verbs: ["read"],
        effect: "auto_allow",
        mode: "per_request",
      }).success,
    ).toBe(false);
  });

  it("rejects unknown effects and verbs", () => {
    expect(
      approvalRuleSchema.safeParse({
        methods: ["GET"],
        resource_pattern: "*",
        verbs: ["execute"],
        effect: "auto_allow",
        mode: "per_request",
      }).success,
    ).toBe(false);

    expect(
      approvalRuleSchema.safeParse({
        methods: ["GET"],
        resource_pattern: "*",
        verbs: ["read"],
        effect: "prompt",
        mode: "per_request",
      }).success,
    ).toBe(false);
  });
});

describe("setServiceApprovalConfigSchema", () => {
  it("accepts granular rules and default_effect", () => {
    const result = setServiceApprovalConfigSchema.safeParse({
      approval_mode: "per_request",
      default_effect: "auto_allow",
      rules: [
        {
          methods: [],
          resource_pattern: "*",
          verbs: ["destructive"],
          effect: "require_approval",
          mode: "per_request",
        },
      ],
    });

    expect(result.success).toBe(true);
  });

  it("requires at least one field", () => {
    expect(setServiceApprovalConfigSchema.safeParse({}).success).toBe(false);
  });

  it("caps rule count at fifty", () => {
    const rules = Array.from({ length: 51 }, () => ({
      methods: [],
      resource_pattern: "*",
      verbs: ["write"],
      effect: "require_approval",
      mode: "per_request",
    }));

    expect(
      setServiceApprovalConfigSchema.safeParse({ rules }).success,
    ).toBe(false);
  });
});
