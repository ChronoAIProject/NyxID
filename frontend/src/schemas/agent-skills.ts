import { z } from "zod";
const pinSchema = z.object({
  source: z.literal("ornn"),
  skill_id: z.string(),
  name: z.string(),
  version: z.string(),
  sha256: z.string(),
});
export const agentSkillSchema = pinSchema.extend({
  dependencies: pinSchema.array().default([]),
});
export type AgentSkill = z.infer<typeof agentSkillSchema>;
export const agentSkillsSchema = z.object({
  revision: z.number(),
  skills: agentSkillSchema.array(),
  metadata: z.record(
    z.string(),
    z.object({ description: z.string(), size_bytes: z.number() }),
  ),
});
export const skillSearchSchema = z.object({
  items: z
    .object({ id: z.string(), name: z.string(), description: z.string() })
    .array(),
  page: z.number(),
  total_pages: z.number(),
});
export const skillVersionsSchema = z.object({
  page: z.number().default(1),
  total_pages: z.number().default(1),
  items: z
    .object({
      version: z.string(),
      sha256: z.string(),
      deprecated: z.boolean(),
    })
    .array(),
});
export const skillPreviewSchema = z.object({
  reference: agentSkillSchema,
  description: z.string(),
  size_bytes: z.number(),
});

export const learningProposalSchema = z.object({
  id: z.string(),
  agent_id: z.string(),
  status: z.string(),
  revision: z.number().default(0),
  config_revision: z.number(),
  agent_skills_revision: z.number(),
  evidence_count: z.number(),
  body_bytes: z.number(),
  created_at: z.string(),
  updated_at: z.string(),
  draft: z.object({
    schema_version: z.number().default(1),
    kind: z.enum(["new", "improve", "none"]).default("new"),
    name: z.string(),
    description: z.string(),
    skill_md: z.string(),
    rationale: z.string(),
    safety_notes: z.string(),
    files: z.array(z.object({ path: z.string(), content: z.string() })).default([]),
    base_skill: z.unknown().optional(),
  }).optional(),
});
export const learningProposalsSchema = z.object({ proposals: learningProposalSchema.array() });
export const learningStatusSchema = z.object({
  enabled: z.boolean(),
  config: z
    .object({
      threshold: z.number().optional(),
      learning_epoch: z.number().optional(),
      config_revision: z.number().optional(),
      eligible_count: z.number().optional(),
      last_run_at: z.string().nullable().optional(),
      last_success_at: z.string().nullable().optional(),
      last_error_code: z.string().nullable().optional(),
    })
    .nullable()
    .optional(),
});
export const learningApprovalSchema = z.object({
  status: z.string(),
  acknowledgement: z
    .object({ acknowledgement_id: z.string() })
    .optional(),
});
