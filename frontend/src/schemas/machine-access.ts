import { z } from "zod";
import { machineConfigurationSchema } from "./machines";
export const MACHINE_CONTEXT_POLICY = "machine-context-mode-v1";
export const capabilitiesSchema = z.object({
  shell: z.boolean(),
  files: z.boolean(),
  browser: z.boolean(),
  computer: z.boolean(),
  developer_browser: z.boolean(),
});
export const machineAccessSchema = z.object({
  node_id: z.string(),
  name: z.string(),
  revision: z.number(),
  protocol_v2: z.boolean(),
  capabilities: capabilitiesSchema,
  ceiling: capabilitiesSchema,
  legacy: z.boolean(),
  can_edit: z.boolean().default(false),
  assignment_revision: z.number().int().positive(),
  membership: z.boolean(),
  capability_edit_enabled: z.boolean(),
  context_setup_enabled: z.boolean(),
  context_policy_version: z.literal(MACHINE_CONTEXT_POLICY),
  capability_edit_reason: z.string().nullable(),
  context_setup_reason: z.string().nullable(),
  authorization_applied: z.boolean(),
  machine_configuration: machineConfigurationSchema.nullable(),
  environment_applied: z.boolean(),
  availability_reason: z.string().nullable(),
  mode: z.enum(["shared_legacy", "separated", "selection_required"]).optional(),
  selection_required: z.boolean().optional(),
  selection_reason: z.string().nullish(),
  separation_choice: z
    .enum(["automatic", "require_separated", "shared_legacy"])
    .nullish(),
  execution_note: z.string().optional(),
  separated_setup_note: z.string().nullish(),
  separated: z
    .object({
      available: z.boolean(),
      landlock_abi: z.number().nullable(),
      reason: z.string().nullable(),
    })
    .nullish(),
  revocation_pending: z.boolean().optional(),
  delivery_pending: z.boolean().optional(),
  quarantine_pending: z.boolean().optional(),
  execution_revocation_pending: z.boolean().optional(),
  revocation_initialization_pending: z.boolean().optional(),
  revocation_expires_at: z.string().nullish(),
  revocation_diagnostic: z
    .enum(["revocation_deadline_invalid", "authority_timing_upgrade_required"])
    .nullish(),
  saved_login_ids: z.array(z.string()).nullable(),
});
export const machineSelectionSchema = z
  .object({
    expected_revision: z.number().int().positive(),
    capabilities: capabilitiesSchema,
    saved_login_ids: z.array(z.string()).max(64).nullable(),
  })
  .refine(
    (v) =>
      v.capabilities.browser ||
      (!v.capabilities.computer && !v.capabilities.developer_browser),
    {
      message: "Browser is required for computer and developer browser access",
      path: ["capabilities", "browser"],
    },
  );
export const machineContextPrepareSchema = z.strictObject({
  mode: z.enum(["separated", "shared_legacy"]),
  policy_version: z.literal(MACHINE_CONTEXT_POLICY),
  acknowledge_separation: z.literal(true),
});
export const machineContextReviewSchema = z.object({
  review_id: z.string().min(1),
  agent_id: z.string().min(1),
  node_id: z.string().min(1),
  status: z.enum(["pending", "approved", "committed", "failed"]),
  mode: z.enum(["separated", "shared_legacy"]),
  policy_version: z.literal(MACHINE_CONTEXT_POLICY),
  expected_revision: z.number().int().positive(),
  capabilities: capabilitiesSchema,
  saved_login_ids: z.array(z.string()).nullable(),
  previous_mode: z.enum(["selection_required", "separated", "shared_legacy"]),
  capabilities_added: z.array(z.string()),
  capabilities_removed: z.array(z.string()),
  saved_login_ids_added: z.array(z.string()),
  saved_login_ids_removed: z.array(z.string()),
  warning: z.string(),
  storage_note: z.string(),
  expires_at: z.string().datetime({ offset: true }),
  applicable: z.boolean(),
  applicability_reason: z.string().nullable(),
  committed_revision: z.number().int().positive().nullable(),
  error_code: z.string().nullable(),
  machine_configuration_target: z
    .object({
      revision: z.number().int().positive(),
      digest: z.string().regex(/^[a-f0-9]{64}$/),
      runtime_generation: z.number().int().nonnegative(),
      credential_epoch: z.number().int().nonnegative(),
    })
    .nullable(),
  authorization_applied: z.boolean(),
  machine_configuration: machineConfigurationSchema.nullable(),
  environment_applied: z.boolean(),
});
export type MachineContextPrepare = z.infer<typeof machineContextPrepareSchema>;
export type MachineContextReview = z.infer<typeof machineContextReviewSchema>;
export const machineContextSchema = z.object({
  context_id: z.string().min(1),
  agent_id: z.string().min(1),
  agent_name: z.string().min(1),
  actor_label: z.string().min(1),
  group_id: z.string().nullable(),
  generation: z.number().int().positive(),
});
export type MachineAccess = z.infer<typeof machineAccessSchema>;
export type MachineSelection = z.infer<typeof machineSelectionSchema>;
export type MachineContext = z.infer<typeof machineContextSchema>;

export const SEPARATED_SETUP_NOTE =
  "To enable separate workspaces and browsers, use a Linux machine with Landlock ABI 6+, separate OS users and a managed browser installation, then approve an owner action card. Browser permissions are not required for configured shared commands and file tools.";
