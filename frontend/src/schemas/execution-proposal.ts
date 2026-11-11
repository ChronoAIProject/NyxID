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
      context_scope: z
        .enum(["current_conversation", "private_conversations", "same_audience"])
        .optional(),
    }),
  ),
  grant_id: z.string().nullable(),
  failure_code: z.string().nullable(),
  result_text: z.string().nullable().optional(),
});
export type ExecutionProposal = z.infer<typeof executionProposalSchema>;

export function executionContextLabel(scope?: string | null) {
  switch (scope) {
    case "current_conversation":
      return "Only this conversation";
    case "private_conversations":
      return "Across my private conversations";
    case "same_audience":
      return "Across conversations with the same audience";
    default:
      return undefined;
  }
}
