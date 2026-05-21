import { describe, it, expect } from "vitest";
import { updateNotificationSettingsSchema } from "./approvals";

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
