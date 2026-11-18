import { z } from "zod";

export const chainVerifyOutcomeSchema = z.enum(["ok", "broken"]);

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
  severity: z.enum(["info", "action"]).optional(),
  technical_details: z.string().nullable().optional(),
  summary: z.string().min(1),
  detail: z.string().min(1),
  remediation: z.string().min(1),
  detected_at: z.string().min(1),
});

export const securityEffectDiagnosticsSchema = z.object({
  counts: z.array(
    z.object({
      kind: z.string().min(1),
      state: z.string().min(1),
      count: z.number().int().nonnegative(),
      at_least: z.boolean(),
    }),
  ),
  oldest_pending_at: z.string().nullable().optional(),
  oldest_pending_age_seconds: z.number().int().nonnegative().nullable().optional(),
});

export const chainVerificationResponseSchema = z.object({
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
  security_effects: securityEffectDiagnosticsSchema.optional(),
});

export type ChainVerifyOutcome = z.infer<typeof chainVerifyOutcomeSchema>;
export type ChainVerifyStatus = z.infer<typeof chainVerifyStatusSchema>;
export type StartupDiagnostic = z.infer<typeof startupDiagnosticSchema>;
export type SecurityEffectDiagnostics = z.infer<
  typeof securityEffectDiagnosticsSchema
>;
export type ChainVerificationResponse = z.infer<
  typeof chainVerificationResponseSchema
>;
