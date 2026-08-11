import { z } from "zod";

export const directSkillSchema = z.object({
  slug: z.string().min(1),
  label: z.string().min(1),
});

export const directModelSchema = z.object({
  id: z.string().min(1),
  label: z.string().min(1),
  default: z.boolean(),
});

export const directSkillsSchema = z.array(directSkillSchema);
export const directModelsSchema = z.array(directModelSchema);

export type DirectSkill = z.infer<typeof directSkillSchema>;
export type DirectModel = z.infer<typeof directModelSchema>;
