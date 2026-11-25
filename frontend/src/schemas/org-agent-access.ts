import { z } from "zod";

export const MAX_AGENT_ACCESS_PEOPLE = 128;

const people = z.array(z.uuid()).max(MAX_AGENT_ACCESS_PEOPLE)
  .refine((ids) => ids.length === new Set(ids).size).nullable();

export const orgAccessDraftSchema = z.object({ maintainers: people, users: people });
export type OrgAccessDraft = z.infer<typeof orgAccessDraftSchema>;
export const orgAccessSchema = orgAccessDraftSchema.extend({
  revision: z.number().int().nonnegative(),
  authoring_available: z.boolean(),
});
export type OrgAccess = z.infer<typeof orgAccessSchema>;
