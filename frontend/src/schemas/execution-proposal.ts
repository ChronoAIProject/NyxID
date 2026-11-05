import { z } from "zod";

export const executionReviewSchema = z.object({
  agent: z.string(),
  connection: z.string(),
  operation: z.string(),
  audience: z.string(),
  inputs: z.string().default(""),
  existing_permission: z.string().default(""),
  added_permission: z.string().default(""),
  scope: z.string(),
  reusable_unavailable: z.string().nullable(),
});
export const executionProposalSchema = z.object({
  version: z.number(),
  source: z.string().optional(),
  supported: z.boolean(),
  revision: z.number(),
  state: z.string(),
  can_decide: z.boolean(),
  review_path: z.string(),
  review: executionReviewSchema.nullable(),
  choices: z.array(
    z.object({
      id: z.string(),
      digest: z.string(),
      label: z.string(),
      valid_until: z.string().nullable(),
    }),
  ),
  grant_id: z.string().nullable(),
  failure_code: z.string().nullable(),
  result_text: z.string().nullable().optional(),
});
export type ExecutionProposal = z.infer<typeof executionProposalSchema>;
