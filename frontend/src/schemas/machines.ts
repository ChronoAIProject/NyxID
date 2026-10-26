import { z } from "zod";
import { creditsSchema } from "./credits.ts";

export const REMOTE_MACHINE_PROFILE = "remote-machine";
export const REMOTE_MACHINE_POLICY = "remote-machine-mvp";
export const REMOTE_MACHINE_DATA_POLICY = "remote-machine-data";

// Safe HTTP projections only. Unknown server fields are stripped before a
// response enters Query; never cache bootstrap material or raw job output.
const machineIdSchema = z.string().min(1);
const machineTimeSchema = z.string().datetime({ offset: true });
const machineRevisionSchema = z.number().int().nonnegative();

export const remoteMachineNameSchema = z
  .string()
  .min(1)
  .max(64)
  .regex(/^[a-z0-9-]+$/, "Use 1–64 lowercase letters, numbers or hyphens.");
export const remoteMachineCapabilitiesSchema = z
  .array(z.enum(["shell", "files", "browser", "computer"]))
  .min(1, "Enable at least one capability.")
  .refine(
    (capabilities) =>
      !capabilities.includes("computer") || capabilities.includes("browser"),
    "Computer requires Browser.",
  );
export const remoteMachineHostedConfigurationSchema = z.object({
  name: remoteMachineNameSchema.default("my-remote-machine"),
  capabilities: remoteMachineCapabilitiesSchema.default(["shell", "files"]),
  specialist_id: machineIdSchema.nullable().default(null),
});
export type RemoteMachineHostedConfiguration = z.output<
  typeof remoteMachineHostedConfigurationSchema
>;

export const remoteMachineProfileSchema = z.object({
  profile_version: z.literal(REMOTE_MACHINE_PROFILE),
  cpu_millis: z.literal(2000),
  memory_mib: z.literal(2048),
  workspace_gib: z.literal(20),
  ephemeral_gib: z.null().optional(),
});
export const remoteMachinePlatformReadinessSchema = z.object({
  configuration_ready: z.boolean(),
  manager_started: z.boolean(),
  namespace: z.string().min(1),
  blockers: z.array(z.string()),
  physical_acceptance: z.literal("native_state_only_no_physical_guarantee"),
  lifecycle_basis: z.literal("agent_sandbox_native_observation"),
});
export const remoteMachinePolicySchema = z.object({
  policy_version: z.literal(REMOTE_MACHINE_POLICY),
  data_policy_version: z.literal(REMOTE_MACHINE_DATA_POLICY),
  free_slot_per_personal_account: z.number().int().nonnegative(),
  free_entitlement_expires: z.boolean(),
  free_monthly_runtime_limit_seconds: z.number().int().nonnegative().nullable(),
  paused_workspace_retention: z.string(),
  paused_storage_price_credits: creditsSchema,
  workspace_backup_or_restore: z.boolean(),
  owner_confirmed_delete_required: z.boolean(),
  cleanup_target_hours: z.number().int().positive(),
  job_history_days: z.number().int().positive(),
  operation_history_days: z.number().int().positive(),
  terminal_output_max_retention_seconds: z.number().int().nonnegative(),
  shared_environment_mode: z.string(),
  cross_client_secret_isolation: z.boolean(),
  inbound_network_service: z.boolean(),
  persistent_environment_variables: z.boolean(),
  service_charges_separate: z.boolean(),
  concurrent_jobs: z.number().int().positive(),
  default_command_timeout_seconds: z.number().int().positive(),
  maximum_command_timeout_seconds: z.number().int().positive(),
  job_output_bytes: z.number().int().positive(),
  machine_output_cache_bytes: z.number().int().positive(),
});
export const remoteMachineSchema = z.object({
  node_id: machineIdSchema,
  // Older servers may omit hosted configuration; absence is not an assignment.
  name: remoteMachineNameSchema.optional(),
  capabilities: remoteMachineCapabilitiesSchema.optional(),
  specialist_id: machineIdSchema.nullable().optional(),
  specialist_assignment_state: z
    .enum(["none", "pending", "assigned", "permissions_required", "failed"])
    .optional(),
  specialist_assignment_error: z
    .literal("specialist_assignment_unavailable")
    .nullable()
    .optional(),
  policy_version: z.literal(REMOTE_MACHINE_POLICY),
  data_policy_version: z.literal(REMOTE_MACHINE_DATA_POLICY),
  profile: remoteMachineProfileSchema,
  sandbox_namespace: z.string(),
  sandbox_name: z.string(),
  sandbox_uid: z.string().nullable(),
  pvc_name: z.string().nullable(),
  pvc_uid: z.string().nullable(),
  pod_uid: z.string().nullable(),
  resource_state: z.enum([
    "provisioning",
    "ready",
    "degraded",
    "stopping",
    "paused",
    "resuming",
    "deleting",
    "deleted",
    "reconciling",
    "failed",
  ]),
  billing_state: z.enum([
    "free",
    "pending",
    "ready",
    "suspended",
    "closed",
    "unavailable",
  ]),
  authorization_state: z.enum(["pending", "active", "revoked"]),
  revision: machineRevisionSchema,
  runtime_generation: machineRevisionSchema,
  credential_epoch: machineRevisionSchema,
  active_operation_id: machineIdSchema.nullable(),
  cleanup_state: z.string(),
  stop_reason: z.string().nullable().optional(),
  // Additive owner projection fields; absence on an older server is unknown.
  free_slot: z.boolean().optional(),
  runtime_run_id: machineIdSchema.nullable().optional(),
  funded_until: machineTimeSchema.nullable().optional(),
  last_heartbeat_at: machineTimeSchema.nullable(),
  updated_at: machineTimeSchema,
});
export const remoteMachineOperationSchema = z.object({
  id: machineIdSchema,
  machine_id: machineIdSchema.nullable(),
  action: z.enum(["create", "pause", "resume", "rekey", "delete", "reconcile"]),
  state: z.enum(["accepted", "applying", "reconciling", "succeeded", "failed"]),
  cleanup_state: z.enum([
    "not_required",
    "pending",
    "in_progress",
    "complete",
    "unknown",
  ]),
  retry_count: z.number().int().nonnegative(),
  delete_retry_attempts: z.number().int().nonnegative().nullable().optional(),
  delete_retry_limit: z.number().int().positive().nullable().optional(),
  delete_retry_stopped: z.boolean().optional(),
  error_code: z.string().nullable(),
  error_message: z.string().nullable(),
  evidence_digest: z.string().nullable(),
  next_attempt_at: machineTimeSchema.nullable(),
  deadline_at: machineTimeSchema.nullable(),
  created_at: machineTimeSchema,
  updated_at: machineTimeSchema,
  terminal_at: machineTimeSchema.nullable(),
});
export const remoteMachineDeleteRetryRequestSchema = z
  .object({
    expected_revision: z.number().int().nonnegative(),
    expected_retry_count: z.number().int().nonnegative(),
  })
  .strict();
export const remoteMachineEventSchema = z.object({
  id: machineIdSchema,
  machine_id: machineIdSchema.nullable(),
  operation_id: machineIdSchema,
  kind: z.string(),
  message: z.string(),
  notification: z.boolean(),
  responsibility: z.string(),
  created_at: machineTimeSchema,
});
export const remoteMachineEventPageSchema = z.union([
  z.object({
    events: z.array(remoteMachineEventSchema),
    next_cursor: z.string().nullable().optional(),
  }),
  z
    .array(remoteMachineEventSchema)
    .transform((events) => ({ events, next_cursor: undefined })),
]);
export const remoteMachineQuotaSchema = z.object({
  status: z.enum(["unknown", "insufficient", "headroom_observed"]),
  reason: z.string(),
  operation: z.enum(["create", "retained_volume_resume"]),
  profile_version: z.literal(REMOTE_MACHINE_PROFILE),
  additional_profiles_upper_bound: z.number().int().nonnegative().nullable(),
  observed_at: machineTimeSchema.nullable(),
  valid_until: machineTimeSchema.nullable(),
  capacity_guarantee: z.literal(false),
  quota_usage_freshness: z.literal("controller_reconciliation_time_unknown"),
});
export const remoteMachineFundsSchema = z.object({
  wallet_available: creditsSchema,
  wallet_reserved: creditsSchema,
  pending_debits: creditsSchema,
  pending_expiry: creditsSchema,
  matching_allowance_seconds: z.number().int().nonnegative(),
  available_grants: creditsSchema,
  estimated_seconds: z.string().regex(/^\d+$/),
  shared_account_estimate_only: z.literal(true),
});
export const remoteMachineBillingPreflightSchema = z.object({
  profile_version: z.string().min(1),
  ready: z.boolean(),
  billing_enabled: z.boolean(),
  exact_ready: z.boolean(),
  wallet: z.enum(["missing", "suspended", "unprovisioned", "ready"]),
  subscription: z.enum(["missing", "unverified", "active", "not_entitled"]),
  provider: z.enum(["unconfigured", "unverified", "ready", "unavailable"]),
  pricing: z.enum(["synced", "unavailable"]),
  reason: z.string().nullable(),
  rate_version: machineIdSchema.nullable(),
  credits_per_second: creditsSchema.nullable(),
  funds: remoteMachineFundsSchema.nullable(),
  funds_reason: z.string().nullable(),
  observed_at: machineTimeSchema,
  valid_until: machineTimeSchema,
  reservation_guarantee: z.literal(false),
});
export const remoteMachineEntitlementSchema = z.object({
  eligible: z.boolean(),
  free_slot_available: z.boolean(),
  machine_id: machineIdSchema.nullable(),
  reason: z.string().nullable(),
  policy_version: z.literal(REMOTE_MACHINE_POLICY),
  policy: remoteMachinePolicySchema,
  platform: remoteMachinePlatformReadinessSchema.optional(),
  quota: remoteMachineQuotaSchema.nullable().optional(),
  billing_preflight: remoteMachineBillingPreflightSchema.nullable().optional(),
});
/** Profile preflight or a synchronized runtime offer, never proof of acceptance. */
export const remoteMachineQuoteSchema = z.object({
  policy_version: z.literal(REMOTE_MACHINE_POLICY),
  profile: remoteMachineProfileSchema,
  free_slot_requested: z.boolean(),
  paid_admission: z.string(),
  pricing_status: z.string(),
  valid_until: machineTimeSchema,
  policy: remoteMachinePolicySchema,
  quote_id: machineIdSchema.nullable().optional(),
  rate_version: machineIdSchema.nullable().optional(),
  credits_per_second: creditsSchema.nullable().optional(),
  funding_mode: z.string().optional(),
  no_instance_cap: z.boolean().optional(),
  cumulative_whole_seconds: z.boolean().optional(),
  paused_storage_free: z.boolean().optional(),
  platform: remoteMachinePlatformReadinessSchema.optional(),
  quota: remoteMachineQuotaSchema.nullable().optional(),
  billing_preflight: remoteMachineBillingPreflightSchema.nullable().optional(),
});
export const remoteMachinePaidQuoteSchema = remoteMachineQuoteSchema.extend({
  free_slot_requested: z.literal(false),
  paid_admission: z.literal("quote_acceptance_required"),
  pricing_status: z.literal("synced"),
  quote_id: machineIdSchema,
  rate_version: machineIdSchema,
  credits_per_second: creditsSchema.refine(
    (value) => !value.startsWith("-") && /[1-9]/.test(value),
    "The accepted runtime rate must be positive",
  ),
  funding_mode: z.literal("account_balance"),
  no_instance_cap: z.literal(true),
  cumulative_whole_seconds: z.literal(true),
  paused_storage_free: z.literal(true),
});
export type RemoteMachinePaidQuote = z.infer<
  typeof remoteMachinePaidQuoteSchema
>;
/** This is a currently acceptable offer; only the server can accept and fund it. */
export function isRemoteMachinePaidQuote(
  value: unknown,
  now = Date.now(),
): value is RemoteMachinePaidQuote {
  const parsed = remoteMachinePaidQuoteSchema.safeParse(value);
  return (
    parsed.success &&
    parsed.data.platform?.configuration_ready === true &&
    new Date(parsed.data.valid_until).getTime() > now
  );
}
export const remoteMachineQuoteRequestSchema = z.strictObject({
  profile_version: z
    .literal(REMOTE_MACHINE_PROFILE)
    .default(REMOTE_MACHINE_PROFILE),
  free_slot: z.boolean(),
  machine_id: machineIdSchema.optional(),
});
export const remoteMachineCreateRequestSchema = z
  .strictObject({
    ...remoteMachineHostedConfigurationSchema.shape,
    profile_version: z
      .literal(REMOTE_MACHINE_PROFILE)
      .default(REMOTE_MACHINE_PROFILE),
    free_slot: z.boolean(),
    policy_version: z.literal(REMOTE_MACHINE_POLICY),
    data_policy_version: z.literal(REMOTE_MACHINE_DATA_POLICY),
    acknowledge_shared_environment: z
      .boolean()
      .refine(
        Boolean,
        "Acknowledge the shared environment before creating a Machine",
      ),
    quote_id: machineIdSchema.optional(),
    accept_account_balance: z.boolean().optional(),
  })
  .superRefine((body, context) => {
    if (
      !body.free_slot &&
      (!body.quote_id || body.accept_account_balance !== true)
    ) {
      context.addIssue({
        code: "custom",
        path: ["accept_account_balance"],
        message:
          "Accept a current runtime quote and continuous account-balance charging without an instance cap",
      });
    }
    if (
      body.free_slot &&
      (body.quote_id !== undefined || body.accept_account_balance === true)
    ) {
      context.addIssue({
        code: "custom",
        path: ["quote_id"],
        message: "A free Machine cannot accept a paid runtime quote",
      });
    }
  });
export const remoteMachineActionRequestSchema = z
  .strictObject({
    action: z.enum(["pause", "resume", "rekey", "delete"]),
    profile_version: z.literal(REMOTE_MACHINE_PROFILE).optional(),
    expected_revision: machineRevisionSchema,
    data_policy_version: z.literal(REMOTE_MACHINE_DATA_POLICY).optional(),
    acknowledge_data_loss: z.boolean().optional(),
    policy_version: z.literal(REMOTE_MACHINE_POLICY).optional(),
    acknowledge_shared_environment: z.boolean().optional(),
    quote_id: machineIdSchema.optional(),
    accept_account_balance: z.boolean().optional(),
  })
  .superRefine((body, context) => {
    if (
      (body.quote_id !== undefined || body.accept_account_balance === true) &&
      (body.action !== "resume" ||
        !body.quote_id ||
        body.accept_account_balance !== true)
    ) {
      context.addIssue({
        code: "custom",
        path: ["accept_account_balance"],
        message:
          "Only an explicitly accepted runtime quote can authorize paid resume",
      });
    }
    if (
      body.action === "delete" &&
      (!body.data_policy_version || !body.acknowledge_data_loss)
    ) {
      context.addIssue({
        code: "custom",
        path: ["acknowledge_data_loss"],
        message:
          "Acknowledge the current data policy and irreversible data loss",
      });
    }
    if (
      (body.action === "resume" || body.action === "rekey") &&
      (!body.policy_version ||
        !body.data_policy_version ||
        !body.acknowledge_shared_environment)
    ) {
      context.addIssue({
        code: "custom",
        path: ["acknowledge_shared_environment"],
        message: "Review and accept the current Machine and data policies",
      });
    }
  });
export const remoteMachineResponseSchema = z.object({
  machine: remoteMachineSchema,
});
export const remoteMachineListResponseSchema = z.object({
  machines: z.array(remoteMachineSchema),
  next_cursor: z.string().nullable().optional(),
});
export const remoteMachineOperationResponseSchema = z.object({
  operation: remoteMachineOperationSchema,
});
export const remoteMachineCreateResponseSchema =
  remoteMachineResponseSchema.extend({
    operation: remoteMachineOperationSchema,
    replayed: z.boolean(),
  });

/** Mounted owner P3 projection. Amounts stay exact decimal strings. */
export const remoteMachineRuntimeBillingSchema = z.object({
  node_id: machineIdSchema,
  run_id: machineIdSchema,
  state: z.enum(["accepted", "running", "stop_pending", "stopped", "closed"]),
  rate_version: machineIdSchema,
  accepted_credits_per_second: creditsSchema,
  covered_until: machineTimeSchema,
  cumulative_seconds: z.number().int().nonnegative(),
  settled_credits: creditsSchema,
  reserved_credits: creditsSchema,
  // Pending is a subset of reserved, never an additional charge to sum.
  pending_credits: creditsSchema,
  funds: z.object({
    wallet_available: creditsSchema,
    wallet_reserved: creditsSchema,
    pending_debits: creditsSchema,
    pending_expiry: creditsSchema,
    matching_allowance_seconds: z.number().int().nonnegative(),
    available_grants: creditsSchema,
    estimated_seconds: z.string().regex(/^\d+$/),
    shared_account_estimate_only: z.literal(true),
  }),
  intervals: z.array(
    z.object({
      interval_id: machineIdSchema,
      runtime_generation: machineRevisionSchema,
      rate_version: machineIdSchema,
      covered_from: machineTimeSchema,
      covered_until: machineTimeSchema,
      usage_row_id: machineIdSchema,
      quantity: z.number().int().nonnegative().nullable(),
      settled: z.boolean(),
      reserved_credits: creditsSchema,
      settled_credits: creditsSchema,
      allowance_funded: creditsSchema,
      grant_funded: creditsSchema,
      wallet_funded: creditsSchema,
    }),
  ),
  next_interval_offset: z.number().int().nonnegative().nullable(),
  attribution: z.literal("machine_runtime"),
  service_costs_included: z.literal(false),
});

/** Safe run summaries mapped from P3 runs; the model has no closed_at field. */
export const remoteMachineBillingRunSchema = z.object({
  run_id: machineIdSchema,
  rate_version: machineIdSchema,
  accepted_credits_per_second: creditsSchema,
  state: z.enum(["accepted", "running", "stop_pending", "stopped", "closed"]),
  created_at: machineTimeSchema,
  updated_at: machineTimeSchema,
  runtime_generation: machineRevisionSchema.optional(),
});
export const remoteMachineBillingRunsSchema = z
  .object({
    runs: z.array(remoteMachineBillingRunSchema),
    next_offset: z.number().int().nonnegative().nullable(),
  })
  .transform(({ runs, next_offset }) => ({
    runs,
    next_cursor: next_offset === null ? null : String(next_offset),
  }));

export type RemoteMachine = z.infer<typeof remoteMachineSchema>;
export type RemoteMachinePolicy = z.infer<typeof remoteMachinePolicySchema>;
export type RemoteMachineOperation = z.infer<
  typeof remoteMachineOperationSchema
>;
export type RemoteMachineEvent = z.infer<typeof remoteMachineEventSchema>;
export type RemoteMachineQuote = z.infer<typeof remoteMachineQuoteSchema>;
export type RemoteMachineEntitlement = z.infer<
  typeof remoteMachineEntitlementSchema
>;
export type RemoteMachineCreateRequest = z.input<
  typeof remoteMachineCreateRequestSchema
>;
export type RemoteMachineActionRequest = z.infer<
  typeof remoteMachineActionRequestSchema
>;
export type RemoteMachineRuntimeBilling = z.infer<
  typeof remoteMachineRuntimeBillingSchema
>;
export type RemoteMachineBillingRun = z.infer<
  typeof remoteMachineBillingRunSchema
>;

/** Safe UI projections; mounted metadata adapters preserve unknown fields as unknown. IDs never authorize access. */
export const remoteMachineSubjectSchema = z.discriminatedUnion("kind", [
  z.object({
    kind: z.literal("assistant"),
    user_id: machineIdSchema,
    agent_id: machineIdSchema.nullable(),
    conversation_id: machineIdSchema.nullable(),
  }),
  z.object({
    kind: z.literal("oauth"),
    user_id: machineIdSchema,
    oauth_client_id: machineIdSchema,
  }),
  z.object({
    kind: z.literal("api_key"),
    user_id: machineIdSchema,
    api_key_id: machineIdSchema,
    api_key_credential_id: machineIdSchema.nullable(),
  }),
]);
export const remoteMachineCancellationRequesterSchema = z.discriminatedUnion(
  "kind",
  [
    z.object({ kind: z.literal("human_owner"), user_id: machineIdSchema }),
    z.object({
      kind: z.literal("external"),
      principal: remoteMachineSubjectSchema,
    }),
    z.object({ kind: z.literal("system"), reason: z.string().min(1) }),
  ],
);
export const remoteMachineJobSchema = z.object({
  id: machineIdSchema,
  node_id: machineIdSchema,
  subject: remoteMachineSubjectSchema,
  grant_id: machineIdSchema.nullable(),
  runtime_id: machineIdSchema.optional(),
  runtime_generation: machineRevisionSchema.nullable(),
  status: z.string().min(1),
  created_at: machineTimeSchema,
  started_at: machineTimeSchema.nullable(),
  finished_at: machineTimeSchema.nullable(),
  expires_at: machineTimeSchema,
  exit_code: z.number().int().nullable(),
  stop_reason: z.string().nullable(),
  duration_ms: z.number().int().nonnegative().nullable().optional(),
  cancellation_requested_by: remoteMachineCancellationRequesterSchema
    .nullable()
    .optional(),
  cancellation_requested_at: machineTimeSchema.nullable().optional(),
  retain_until: machineTimeSchema.nullable().optional(),
  output_query_available: z.boolean().optional(),
  output_availability: z.enum([
    "query_required",
    "unavailable",
    "available",
    "expired",
    "lost",
  ]),
  output_expires_at: machineTimeSchema.nullable(),
  output_reason: z.string().nullable(),
  services: z.array(z.object({ id: machineIdSchema, slug: z.string().min(1) })),
  cancellation_available: z.boolean().optional(),
});
export const remoteMachineJobsSchema = z.object({
  jobs: z.array(remoteMachineJobSchema),
  next_cursor: z.string().nullable(),
});
/** Actual current owner history projection. Missing execution details stay unknown. */
export const remoteMachineOwnerHistorySchema = z.object({
  jobs: z.array(
    z.object({
      id: machineIdSchema,
      job_id: machineIdSchema.optional(),
      node_id: machineIdSchema,
      runtime_id: machineIdSchema.optional(),
      runtime_generation: machineRevisionSchema.nullable(),
      state: z.string().min(1),
      created_at: machineTimeSchema,
      expires_at: machineTimeSchema,
      finished_at: machineTimeSchema.nullable(),
      exit_code: z.number().int().nullable().optional(),
      stop_reason: z.string().nullable().optional(),
      duration_ms: z.number().int().nonnegative().nullable().optional(),
      cancellation_requested_by: remoteMachineCancellationRequesterSchema
        .nullable()
        .optional(),
      cancellation_requested_at: machineTimeSchema.nullable().optional(),
      retain_until: machineTimeSchema.nullable().optional(),
      binding: z.discriminatedUnion("kind", [
        z.object({
          kind: z.literal("external"),
          principal: remoteMachineSubjectSchema,
          grant_id: machineIdSchema,
          grant_revision: machineRevisionSchema,
        }),
        z.object({
          kind: z.literal("assistant"),
          agent_id: machineIdSchema.nullable().optional(),
          conversation_id: machineIdSchema.nullable().optional(),
          legacy: z.boolean().optional(),
        }),
      ]),
      services: z.array(
        z.object({ id: machineIdSchema, slug: z.string().min(1) }),
      ),
      output_query_available: z.boolean().optional(),
      output_availability: z.enum(["query_required", "unavailable"]).optional(),
      cancellation_available: z.boolean().optional(),
      output_persisted: z.literal(false),
    }),
  ),
  next_cursor: z.string().nullable(),
});
/** Raw Node output is transient component state, never Query or mutation data. */
export const remoteMachineJobOutputSchema = z.object({
  job_id: machineIdSchema,
  status: z.string().min(1).optional().default("unknown"),
  exit_code: z.number().int().nullable().optional().default(null),
  stdout: z.string().optional(),
  stderr: z.string().optional(),
  stdout_bytes: z.number().int().nonnegative().optional(),
  stderr_bytes: z.number().int().nonnegative().optional(),
  output_offset: z.number().int().nonnegative().optional(),
  stderr_offset: z.number().int().nonnegative().optional(),
  truncated: z.boolean().optional(),
  timed_out: z.boolean().optional(),
  duration_ms: z.number().int().nonnegative().optional(),
  output_available: z.boolean().optional(),
  output_unavailable_reason: z.string().nullable().optional(),
});
export const remoteMachineConfirmationSchema = z.object({
  id: machineIdSchema,
  node_id: machineIdSchema,
  subject: remoteMachineSubjectSchema,
  grant_id: machineIdSchema,
  grant_revision: machineRevisionSchema,
  assignment_revision: machineRevisionSchema,
  runtime_id: machineIdSchema.optional(),
  runtime_generation: machineRevisionSchema.nullable(),
  operation: z.string().min(1),
  parameters_digest: z.string().min(1),
  // Server-owned, redacted summaries only; never full command/stdin/env/output.
  safe_parameters: z
    .array(z.object({ name: z.string(), value: z.string() }))
    .default([]),
  review_parameters_available: z.boolean().optional(),
  review_content_available: z.boolean().optional(),
  status: z.enum([
    "pending",
    "approved",
    "declined",
    "expired",
    "consumed",
    "revoked",
  ]),
  created_at: machineTimeSchema,
  expires_at: machineTimeSchema,
  approved_at: machineTimeSchema.nullable(),
  approved_until: machineTimeSchema.nullable(),
});
export const remoteMachineConfirmationsSchema = z.object({
  confirmations: z.array(remoteMachineConfirmationSchema),
  next_cursor: z.string().nullable().optional(),
});
export const remoteMachineOwnerConfirmationsSchema = z.object({
  next_cursor: z.string().nullable().optional(),
  confirmations: z.array(
    z.object({
      id: machineIdSchema,
      node_id: machineIdSchema,
      principal: remoteMachineSubjectSchema,
      grant_id: machineIdSchema,
      grant_revision: machineRevisionSchema,
      assignment_revision: machineRevisionSchema,
      runtime_id: machineIdSchema.optional(),
      runtime_generation: machineRevisionSchema.nullable(),
      operation: z.string().min(1),
      parameters_digest: z.string().min(1),
      safe_parameters: z
        .array(z.object({ name: z.string(), value: z.string() }))
        .default([]),
      review_parameters_available: z.boolean().optional(),
      review_content_available: z.boolean().optional(),
      state: z.enum([
        "pending",
        "approved",
        "declined",
        "expired",
        "consumed",
        "revoked",
      ]),
      created_at: machineTimeSchema,
      expires_at: machineTimeSchema,
      decided_at: machineTimeSchema.nullable(),
      owner_only: z.literal(true),
    }),
  ),
});
export const remoteMachineConfirmationDecisionSchema = z.strictObject({
  approve: z.boolean(),
  expected_grant_revision: machineRevisionSchema,
  expected_runtime_generation: machineRevisionSchema,
  parameters_digest: z.string().min(1),
});
export const remoteMachineBoundConfirmationResponseSchema = z.object({
  confirmation: remoteMachineConfirmationSchema,
});
export const remoteMachineJobCancellationSchema = z.object({
  job_id: machineIdSchema,
  status: z.string().min(1),
  cancellation_acknowledged: z.boolean().optional(),
  stop_acknowledged: z.boolean(),
});
export type RemoteMachineSubject = z.infer<typeof remoteMachineSubjectSchema>;
export type RemoteMachineJob = z.infer<typeof remoteMachineJobSchema>;
export type RemoteMachineJobs = z.infer<typeof remoteMachineJobsSchema>;
export type RemoteMachineJobOutput = z.infer<
  typeof remoteMachineJobOutputSchema
>;
export type RemoteMachineConfirmation = z.infer<
  typeof remoteMachineConfirmationSchema
>;
export type RemoteMachineConfirmationDecision = z.infer<
  typeof remoteMachineConfirmationDecisionSchema
>;

export const machineChoicesSchema = z.object({
  separation: z.enum(["automatic", "require_separated", "shared_legacy"]).nullish(),
  legacy_warning_acknowledged: z.boolean().optional(),
  owner_id: z.string().uuid().nullable().optional(),
  name: z
    .string()
    .min(1)
    .max(64)
    .regex(/^[a-z0-9-]+$/, "Use lowercase letters, numbers and hyphens"),
  where: z.enum(["this_computer", "vm", "docker"]),
  capabilities: z
    .array(z.enum(["shell", "files", "browser", "computer"]))
    .min(1)
    .max(4),
  grant_to: z.string().nullable(),
  automatic_updates: z.boolean().nullable().optional(),
}).refine((choices) => choices.separation !== "shared_legacy" || choices.legacy_warning_acknowledged === true, {
  path: ["legacy_warning_acknowledged"], message: "Acknowledge the shared legacy warning before continuing",
});
export type MachineChoices = z.infer<typeof machineChoicesSchema>;
export interface MachineProfile {
  separation_choice?: "automatic" | "require_separated" | "shared_legacy" | null;
  separation_evidence?: {
    version: number;
    os: string;
    landlock_abi?: number | null;
    separate_users?: boolean | null;
    managed_browser_configured?: boolean | null;
    command_seccomp?: boolean | null;
    landlock_enforced?: boolean | null;
    managed_browser_sandbox?: boolean | null;
    managed_browser_policy?: boolean | null;
    credential_custody?: boolean | null;
    probe_timed_out?: boolean;
    browser_sandbox_unobservable_in_container?: boolean;
  } | null;
  version: number;
  shell: boolean;
  files: boolean;
  computer: boolean;
  browser?: boolean | null;
  authority_versions?: number[];
  separated?: {
    available: boolean;
    landlock_abi: number | null;
    reason: string | null;
  } | null;
  os: string;
  arch: string;
  roots: string[];
  computer_mode: "standard" | "unrestricted";
  cua_version: string | null;
  computer_tools: string[];
  computer_ready: boolean;
  computer_permissions?: {
    screen_recording: boolean | null;
    accessibility: boolean | null;
  } | null;
  browser_isolated: boolean;
  commands_isolated?: boolean | null;
  saved_login_ready: boolean;
  installation?: "container" | "native";
  updater_ready?: boolean;
}

/** Display only. The server remains authoritative for assignment selection.
 * Raw node profiles may carry the old support boolean without v4 evidence. */
export function machineSeparationSupport(machine: MachineProfile) {
  const evidence = machine.separation_evidence;
  const unavailable = (reason: string) => ({ available: false, reason });
  if (!evidence) return unavailable("separated_capabilities_not_reported");
  if (evidence.version !== 4) return unavailable("separated_probe_version_unsupported");
  if (evidence.os !== "linux") return unavailable("separated_requires_linux");
  if (evidence.browser_sandbox_unobservable_in_container === true)
    return unavailable("separated_browser_sandbox_unobservable_in_container");
  if (evidence.managed_browser_policy === false) return unavailable("separated_browser_policy_not_effective");
  if (evidence.probe_timed_out) return unavailable("separated_probe_timeout");
  if (!Number.isInteger(evidence.landlock_abi) || (evidence.landlock_abi ?? 0) < 6)
    return unavailable("separated_requires_landlock_abi_6");
  for (const [observed, reason] of [
    [evidence.separate_users, "separated_requires_separate_users"],
    [evidence.managed_browser_configured, "separated_requires_managed_browser"],
    [evidence.command_seccomp, "separated_seccomp_probe_failed"],
    [evidence.landlock_enforced, "separated_landlock_probe_failed"],
    [evidence.managed_browser_policy, "separated_browser_policy_not_effective"],
    [evidence.managed_browser_sandbox, "separated_browser_sandbox_probe_failed"],
  ] as const) {
    if (observed === false) return unavailable(reason);
    if (observed !== true) return unavailable("separated_probe_evidence_incomplete");
  }
  if (evidence.credential_custody !== true)
    return unavailable("node_custody_unverified");
  if (!machine.authority_versions?.includes(2))
    return unavailable("machine_authority_upgrade_required");
  return { available: true, reason: null };
}
export interface MachineSetup {
  selection_reason?: string | null;
  id: string;
  choices: MachineChoices;
  status: string;
  hostname: string | null;
  os: string | null;
  ip: string | null;
  conversation_id: string | null;
  expires_at: string;
  machine: MachineProfile | null;
}
export const MACHINE_SAFETY =
  "Recommended: use the machine container or a VM set up with --separate-users. Agents act with that OS user's full access, and prompt injection is possible. Workspace roots constrain file tools and working directories; shell commands can access anything that user can.";
export const SINGLE_USER_SHELL_WARNING =
  "Not isolated: agent commands can read this node's stored credentials, signing secret and node token, including its config and local credential store. File-tool workspace limits do not constrain the shell. You may proceed; prefer the machine container or a VM set up with --separate-users.";
export const SHARED_LEGACY_WARNING = "Shared legacy access gives agent commands the full access of their OS user. A misbehaving or prompt-injected agent may read node tokens, signing secrets and stored credentials accessible to that user, including the node config and local credential store. Workspace roots constrain file tools and working directories, not the shell. Supported separated contexts still share the host kernel and retain kernel-exploit and supervisor-compromise risks; they are not a VM boundary. Use a separate machine container or VM per agent for stronger isolation; containers also share their host kernel.";
export const SINGLE_USER_WARNING =
  "Agent commands run as the same user as the browser on this machine. A misbehaving or prompt-injected agent could read what is typed. Use the machine container or a VM with separate browser and agent users for better isolation.";

/** Shell quoting is applied to every value from a page, API or runtime config. */
export function shellQuote(value: string): string {
  if (value.includes("\0")) throw new Error("Invalid command argument");
  return `'${value.replaceAll("'", "'\\''")}'`;
}
export function machineSetupCommand(
  choices: MachineChoices,
  token: string,
  wsUrl: string,
  version?: string,
  updaterImage?: string,
): string {
  if (!/^nyx_nreg_[a-f0-9]{64}$/.test(token))
    throw new Error("Invalid setup credential");
  const selection = choices.separation ?? "automatic";
  if (!["automatic", "require_separated", "shared_legacy"].includes(selection) || (selection === "shared_legacy" && choices.legacy_warning_acknowledged !== true))
    throw new Error("Review and explicitly acknowledge the machine separation choice");
  const cliSelection = selection.replaceAll("_", "-");
  const args = choices.capabilities
    .map((capability) => `--${capability}`)
    .join(" ");
  if (choices.where === "docker") {
    if (!version || !/^\d+\.\d+\.\d+(?:-[a-zA-Z0-9.-]+)?$/.test(version))
      throw new Error(
        "Server release version is unavailable. Reload before creating the Docker command.",
      );
    const profileUrl = new URL("/machine-seccomp.json", window.location.origin)
      .href;
    return `(nyx_machine_dir=$(mktemp -d) && trap 'rm -rf "$nyx_machine_dir"' EXIT && curl -fsSL ${shellQuote(profileUrl)} -o "$nyx_machine_dir/seccomp.json" && docker run -d --security-opt "seccomp=$nyx_machine_dir/seccomp.json" --name ${shellQuote(choices.name)} --restart unless-stopped --shm-size=1g --label ${shellQuote(`dev.nyxid.machine=${choices.name}`)} -v ${shellQuote(`${choices.name}-nyxid-update:/var/lib/nyxid-machine-update`)} -v ${shellQuote(`${choices.name}-identity:/var/lib/nyxid-machine`)} -v ${shellQuote(`${choices.name}-workspace:/workspace`)} -e ${shellQuote(`NYXID_MACHINE_SEPARATION=${cliSelection}`)} -e ${shellQuote(`NYXID_NODE_TOKEN=${token}`)} -e ${shellQuote(`NYXID_NODE_URL=${wsUrl}`)} ghcr.io/chronoaiproject/nyxid/nyxid-node-machine:${version} ${args} && ${machineCompanionCommand(choices.name, version, updaterImage)})`;
  }
  const installer =
    "https://raw.githubusercontent.com/ChronoAIProject/NyxID/main/skills/nyxid/scripts/install.sh";
  return `(command -v nyxid >/dev/null 2>&1 || bash -c "$(curl -fsSL ${installer})") && PATH="$HOME/.local/bin:$PATH" nyxid node setup --separation ${shellQuote(cliSelection)} --token ${shellQuote(token)} --url ${shellQuote(wsUrl)} ${args}`;
}

export interface MachineUpdateStatus {
  node_id: string;
  current_version: string;
  target_version: string;
  update_available: boolean;
  updater_ready: boolean;
  updater?: {
    version: string;
    digest: string | null;
    target_version: string | null;
    phase: "current" | "pending" | "failed" | "legacy";
    code: string | null;
  } | null;
  updater_guidance?: string | null;
  installation: "container" | "native";
  automatic: boolean;
  phase: string;
  code: string | null;
  guidance?: string | null;
  settings_path: string;
}

function validateUpdate(name: string, version: string) {
  if (
    !/^[a-zA-Z0-9][a-zA-Z0-9_.-]{0,127}$/.test(name) ||
    !/^\d+\.\d+\.\d+(?:-[a-zA-Z0-9.-]+)?$/.test(version)
  )
    throw new Error(
      "Provide a valid Docker container name and server release.",
    );
}

function verifiedUpdaterImage(image?: string) {
  if (
    !image ||
    !/^ghcr\.io\/chronoaiproject\/nyxid\/nyxid-machine-updater@sha256:[a-f0-9]{64}$/.test(
      image,
    )
  )
    throw new Error("Verifying the updater image, try again shortly.");
  return image;
}

export function machineCompanionCommand(
  name: string,
  version: string,
  image?: string,
) {
  validateUpdate(name, version);
  return `docker run -d --name ${shellQuote(`${name}-updater`)} --restart unless-stopped --label ${shellQuote(`dev.nyxid.machine.updater=${name}`)} --read-only --tmpfs /tmp:rw,noexec,nosuid,size=16m --cap-drop=ALL --security-opt=no-new-privileges --mount type=bind,src=/var/run/docker.sock,dst=/var/run/docker.sock --mount ${shellQuote(`type=volume,src=${name}-nyxid-update,dst=/var/lib/nyxid-machine-update`)} ${verifiedUpdaterImage(image)} watch ${shellQuote(name)}`;
}

export function machineCompanionReplacementCommand(
  name: string,
  version: string,
  image?: string,
) {
  const companion = machineCompanionCommand(name, version, image);
  return `docker rm -f ${shellQuote(`${name}-updater`)} && ${companion}`;
}

export function machineMigrationCommand(
  name: string,
  version: string,
  image?: string,
) {
  validateUpdate(name, version);
  return `docker run --rm --read-only --tmpfs /tmp:rw,noexec,nosuid,size=16m --cap-drop=ALL --security-opt=no-new-privileges --mount type=bind,src=/var/run/docker.sock,dst=/var/run/docker.sock --mount ${shellQuote(`type=volume,src=${name}-nyxid-update,dst=/var/lib/nyxid-machine-update`)} ${verifiedUpdaterImage(image)} bootstrap ${shellQuote(name)} ${shellQuote(version)}`;
}
