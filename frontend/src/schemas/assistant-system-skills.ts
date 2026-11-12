import { z } from "zod";
import { skillPinSchema } from "./agent-skills";

export const systemSkillSchema = skillPinSchema.omit({ source: true }).extend({
  dependencies: skillPinSchema.array(),
  description: z.string(),
  size_bytes: z.number(),
});
export const systemSkillsSchema = z.object({
  revision: z.number().int().nonnegative(),
  skills: systemSkillSchema.array().max(8),
  updated_by: z.string().nullable(),
  updated_at: z.string().nullable(),
});
export const systemSkillsSelectionSchema = z
  .object({
    expected_revision: z.number().int().nonnegative(),
    skills: z
      .array(
        z
          .object({
            skill: z.string().min(1),
            version: z
              .string()
              .regex(
                /^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$/,
                "Choose an exact major.minor version.",
              ),
          })
          .strict(),
      )
      .max(8),
  })
  .strict();
export type SystemSkills = z.infer<typeof systemSkillsSchema>;
export type SystemSkillsSelection = z.infer<typeof systemSkillsSelectionSchema>;
