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
