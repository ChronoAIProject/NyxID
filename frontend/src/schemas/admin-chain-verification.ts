import { z } from "zod";

export const chainVerifyOutcomeSchema = z.enum(["ok", "broken"]);

export const chainVerifyStatusSchema = z.object({
  chain: z.enum(["audit_log", "billing_ledger"]),
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

export const chainVerificationResponseSchema = z.object({
  chains: z.array(chainVerifyStatusSchema),
});

export type ChainVerifyOutcome = z.infer<typeof chainVerifyOutcomeSchema>;
export type ChainVerifyStatus = z.infer<typeof chainVerifyStatusSchema>;
export type ChainVerificationResponse = z.infer<
  typeof chainVerificationResponseSchema
>;
