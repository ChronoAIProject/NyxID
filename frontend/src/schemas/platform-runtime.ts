import { z } from "zod";
import { parseCredits } from "@/lib/credits";

const version = z.number().int().positive().max(Number.MAX_SAFE_INTEGER);
export const publicCreditsSchema = z
  .string()
  .max(52)
  .regex(/^\d+(?:\.\d{1,12})?$/, "Use an exact decimal with up to 12 places")
  .refine((value) => {
    try {
      return parseCredits(value) > 0n;
    } catch {
      return false;
    }
  }, "Credits must be greater than zero");
export const publicSetupSchema = z
  .object({
    source: z.literal("nyxagent"),
    profile: z.string().max(128),
    total_daily_credits: publicCreditsSchema,
    sender_daily_credits: publicCreditsSchema,
    sponsor_owner_user_id: z.uuid(),
  })
  .superRefine((value, ctx) => {
    try {
      if (
        parseCredits(value.sender_daily_credits) >
        parseCredits(value.total_daily_credits)
      )
        ctx.addIssue({
          code: "custom",
          path: ["sender_daily_credits"],
          message: "Per-person quota must not exceed the total budget",
        });
    } catch {
      /* Field validation reports malformed amounts. */
    }
  });
export type PublicSetup = z.infer<typeof publicSetupSchema>;
export const publicReadinessSchema = z
  .object({
    status: z.enum(["ready", "faq_ready", "unavailable"]),
    reason: z
      .enum([
        "mode_disabled",
        "manager_unavailable",
        "emergency_stopped",
        "personal_disabled",
        "public_disabled",
        "owner_unavailable",
        "transport_unavailable",
        "origin_revoked",
        "engine_unavailable",
        "unsupported_input",
        "insufficient_funds",
        "capacity_limited",
        "public_not_ready",
        "public_quota_limited",
        "public_turn_failed",
        "runtime_uninitialized",
        "sponsor_unavailable",
        "profile_unavailable",
        "proof_expired",
        "public_contract_unsupported",
        "public_authority_rejected",
      ])
      .nullable(),
    checked_at: z.iso.datetime({ offset: true }).nullable(),
    expires_at: z.iso.datetime({ offset: true }).nullable(),
    evidence_digest: z.string().nullable(),
    policy_digest: z.string().nullable().optional(),
  })
  .superRefine((value, ctx) => {
    if (
      value.status === "ready" &&
      (!value.checked_at ||
        !value.expires_at ||
        !value.evidence_digest ||
        value.reason !== null)
    )
      ctx.addIssue({
        code: "custom",
        message: "Readiness evidence is incomplete",
      });
    if (
      value.status === "unavailable" &&
      (!value.reason || value.evidence_digest !== null)
    )
      ctx.addIssue({
        code: "custom",
        message: "Unavailable readiness cannot authorize execution",
      });
  });
export type PublicReadiness = z.infer<typeof publicReadinessSchema>;
export const managerRuntimeSchema = z
  .object({
    contract_version: z.literal(1),
    supported: z.boolean(),
    state: z
      .enum([
        "setup_incomplete",
        "connection_repair_required",
        "paused",
        "active",
        "retired",
      ])
      .optional(),
    runtime: z
      .object({
        activated_policy_digest: z.string().nullable().optional(),
        manager_authority_epoch: version,
        personal_control_revision: version,
        public_policy_revision: version,
        personal_enabled: z.boolean(),
        public_enabled: z.boolean(),
        emergency_stopped: z.boolean(),
        remembered_personal_enabled: z.boolean().nullable().optional(),
        remembered_public_enabled: z.boolean().nullable().optional(),
        public_policy: publicSetupSchema.nullable(),
      })
      .nullable(),
    public_readiness: publicReadinessSchema,
  })
  .refine(
    (value) => value.supported || value.runtime === null,
    "Unsupported runtime must be absent",
  );
export type ManagerRuntime = z.infer<typeof managerRuntimeSchema>;
export const publicProfilesSchema = z
  .object({
    profiles: z.array(z.object({ id: z.string().min(1), label: z.string() })),
    default_profile: z.string().nullable(),
    readiness: publicReadinessSchema,
  })
  .refine(
    (value) =>
      value.default_profile === null ||
      value.profiles.some((profile) => profile.id === value.default_profile),
    "Default profile must be advertised",
  );
export function runtimePreconditions(
  profileGeneration: number,
  runtime: ManagerRuntime["runtime"],
) {
  return runtime
    ? {
        expected_manager_authority_epoch: runtime.manager_authority_epoch,
        expected_personal_control_revision: runtime.personal_control_revision,
        expected_public_policy_revision: runtime.public_policy_revision,
      }
    : {
        initialize: true as const,
        expected_profile_generation: profileGeneration,
        expected_manager_authority_epoch: 0,
        expected_personal_control_revision: 0,
        expected_public_policy_revision: 0,
      };
}
