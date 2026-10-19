import { expect, it } from "vitest";
import {
  managerRuntimeSchema,
  publicCreditsSchema,
  publicSetupSchema,
  runtimePreconditions,
} from "./platform-runtime";
import { personalLinkPreviewSchema } from "./platform-personal";
const unavailable = {
  status: "unavailable",
  reason: "runtime_uninitialized",
  checked_at: null,
  expires_at: null,
  evidence_digest: null,
};

it("distinguishes supported absent runtime, unsupported runtime, and malformed missing runtime", () => {
  const value = {
    contract_version: 1,
    supported: true,
    runtime: null,
    public_readiness: unavailable,
  };
  expect(managerRuntimeSchema.parse(value).supported).toBe(true);
  expect(
    managerRuntimeSchema.parse({ ...value, supported: false }).runtime,
  ).toBeNull();
  const missing: Partial<typeof value> = { ...value };
  delete missing.runtime;
  expect(managerRuntimeSchema.safeParse(missing).success).toBe(false);
  expect(runtimePreconditions(7, null)).toEqual({
    initialize: true,
    expected_profile_generation: 7,
    expected_manager_authority_epoch: 0,
    expected_personal_control_revision: 0,
    expected_public_policy_revision: 0,
  });
});

it("validates exact decimal strings without float rounding or legacy truncation", () => {
  expect(publicCreditsSchema.parse("1000000000000000.000000000001")).toBe(
    "1000000000000000.000000000001",
  );
  for (const value of [0.1, "0", "-1", "1e2", "0.0000000000001"])
    expect(publicCreditsSchema.safeParse(value).success).toBe(false);
  const setup = {
    source: "nyxagent",
    profile: "nyxagent/chat",
    sponsor_owner_user_id: "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa",
    total_daily_credits: "1.000000000001",
    sender_daily_credits: "1.000000000002",
  };
  expect(publicSetupSchema.safeParse(setup).success).toBe(false);
  expect(
    publicSetupSchema.safeParse({
      ...setup,
      sender_daily_credits: "1.000000000001",
    }).success,
  ).toBe(true);
});

it("rejects fabricated ready without complete server proof", () => {
  expect(
    managerRuntimeSchema.safeParse({
      contract_version: 1,
      supported: true,
      runtime: null,
      public_readiness: { ...unavailable, status: "ready", reason: null },
    }).success,
  ).toBe(false);
});

it("preserves Telegram identifiers beyond JavaScript integer precision", () => {
  const preview = {
    purpose: "personal_nyxbot",
    status: "pending",
    manager: { id: "manager", username: null, display_name: "Bot" },
    sender: { id: "9007199254740993", display_name: null },
    expires_at: "2026-10-15T10:00:00Z",
    invitation_required: true,
    requires_same_chat_confirmation: true,
  };
  expect(personalLinkPreviewSchema.parse(preview).sender.id).toBe(
    "9007199254740993",
  );
  expect(
    personalLinkPreviewSchema.safeParse({
      ...preview,
      sender: { ...preview.sender, id: Number("9007199254740993") },
    }).success,
  ).toBe(false);
});
