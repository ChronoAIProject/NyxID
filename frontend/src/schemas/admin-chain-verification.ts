import { z } from "zod";

export const chainVerifyOutcomeSchema = z.enum(["ok", "broken"]);

const admissionSchema = z.enum(["open", "alarm", "closed"]);
const tierLimitSchema = z.object({
  pending_age_secs: z.number().int().positive(),
  pending_count: z.number().int().positive(),
});
export const auditWitnessDiagnosticsSchema = z.object({
  last_transition: z
    .object({
      at: z.string(),
      reason: z.enum([
        "unwitnessed",
        "ready",
        "stale",
        "backlogged",
        "diverged",
        "unavailable",
      ]),
    })
    .nullable()
    .optional(),
  witness: z.enum([
    "unwitnessed",
    "ready",
    "stale",
    "backlogged",
    "diverged",
    "unavailable",
  ]),
  admission: z.object({
    narrowing: admissionSchema,
    availability: admissionSchema,
    standard: admissionSchema,
    effect: admissionSchema,
  }),
  chain_id: z.string().uuid().nullable(),
  pending_count: z.number().int().nonnegative().nullable(),
  oldest_pending_secs: z.number().int().nonnegative().nullable(),
  last_contact_secs: z.number().int().nonnegative().nullable(),
  local_head_seq: z.number().int().nonnegative().nullable(),
  witnessed_seq: z.number().int().nonnegative().nullable(),
  standard_closes_in_secs: z.number().int().nonnegative().nullable(),
  availability_closes_in_secs: z.number().int().nonnegative().nullable(),
  divergence: z
    .object({
      seq: z.number().int().nonnegative(),
      local_hash_prefix: z.string(),
      witnessed_hash_prefix: z.string(),
    })
    .nullable(),
  limits: z.object({
    contact_secs: z.number().int().positive(),
    pending_age_secs: z.number().int().positive(),
    pending_count: z.number().int().positive(),
    standard: tierLimitSchema,
    availability: tierLimitSchema,
  }),
});

export const externalAuditEvidenceSchema = z.object({
  diagnostics: auditWitnessDiagnosticsSchema.optional(),
  status: z.enum(["unwitnessed", "unverified", "unavailable", "broken"]),
  local_head_seq: z.number().int().nonnegative().nullable().optional(),
  witnessed_seq: z.number().int().nonnegative().nullable().optional(),
  pending_count: z.number().int().nonnegative().nullable().optional(),
  oldest_pending_at: z.string().nullable().optional(),
});

export const chainVerifyStatusSchema = z.object({
  chain: z.enum(["audit_log", "billing_ledger", "billing_accounts"]),
  outcome: chainVerifyOutcomeSchema,
  cursor_seq: z.number().int(),
  head_seq: z.number().int().nullable().optional(),
  checked_entries: z.number().int(),
  last_full_pass_at: z.string().nullable().optional(),
  break_seq: z.number().int().nullable().optional(),
  break_kind: z.string().nullable().optional(),
  break_detail: z.string().nullable().optional(),
  anchor_seq: z.number().int().nullable().optional(),
  anchor_valid: z.boolean().nullable().optional(),
  pre_chain_count: z.number().int().nullable().optional(),
  last_run_at: z.string().min(1),
});

export const startupDiagnosticSchema = z.object({
  code: z.string().min(1),
  summary: z.string().min(1),
  detail: z.string().min(1),
  remediation: z.string().min(1),
  detected_at: z.string().min(1),
});

export const chainVerificationResponseSchema = z.object({
  // An older replica supplies no external proof; absence must never imply success.
  external_evidence: externalAuditEvidenceSchema.default({
    status: "unavailable",
  }),
  chains: z.array(
    chainVerifyStatusSchema.extend({
      chain: z.enum(["audit_log", "billing_ledger"]),
    }),
  ),
  accounts: chainVerifyStatusSchema
    .extend({ chain: z.literal("billing_accounts") })
    .nullable()
    .optional(),
  startup_diagnostics: z.array(startupDiagnosticSchema).default([]),
});

export type ChainVerifyOutcome = z.infer<typeof chainVerifyOutcomeSchema>;
export type ExternalAuditEvidence = z.infer<typeof externalAuditEvidenceSchema>;
export type ChainVerifyStatus = z.infer<typeof chainVerifyStatusSchema>;
export type StartupDiagnostic = z.infer<typeof startupDiagnosticSchema>;
export type ChainVerificationResponse = z.infer<
  typeof chainVerificationResponseSchema
>;
