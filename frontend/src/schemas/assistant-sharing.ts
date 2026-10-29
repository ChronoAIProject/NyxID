import { z } from "zod";

const shareAudienceFields = z.object({
  subject_kind: z.enum(["person", "org"]),
  subject_id: z.uuid().optional(),
  email: z.email().optional(),
  role: z.enum(["manager", "user"]),
});
const validateAudience = (s: z.infer<typeof shareAudienceFields>, ctx: z.RefinementCtx) => {
  if (s.subject_kind === "org" && s.role !== "user") {
    ctx.addIssue({ code: "custom", message: "Organizations receive user access", path: ["role"] });
  }
  const hasId = !!s.subject_id;
  const hasEmail = !!s.email;
  if (s.subject_kind === "person" ? hasId === hasEmail : !hasId || hasEmail) {
    ctx.addIssue({ code: "custom", message: s.subject_kind === "person" ? "Use an ID for an existing person or an email for a new person" : "Organizations require an ID", path: [hasEmail ? "email" : "subject_id"] });
  }
};
export const shareAudienceSchema = shareAudienceFields.superRefine(validateAudience);
const shareAudienceViewSchema = shareAudienceFields.extend({
  slug: z.string().optional(),
  name: z.string().optional(),
}).superRefine((s, ctx) => {
  if (s.subject_kind === "org" && s.role !== "user") {
    ctx.addIssue({ code: "custom", message: "Organizations receive user access", path: ["role"] });
  }
  if (!s.subject_id) {
    ctx.addIssue({ code: "custom", message: "Audience rows require an ID", path: ["subject_id"] });
  }
});
export const sharingFormSchema = z.object({
  enabled: z.boolean(),
  audience: z.array(shareAudienceSchema).max(500),
  shared_access: z.record(z.string(), z.enum(["read", "use", "all"])),
});
export const sharingSchema = sharingFormSchema.extend({
  role: z.enum(["owner", "manager", "user"]),
  version: z.number(),
  revision: z.number(),
  owner_id: z.string(),
  services: z.array(z.object({ id: z.string(), name: z.string() })).default([]),
  audience: z.array(shareAudienceViewSchema).default([]),
  shared_access: z.record(z.string(), z.enum(["read", "use", "all"])).default({}),
});
export const sharedNotesSchema = z.object({
  revision: z.number(),
  notes: z.array(z.object({ id: z.string(), text: z.string() })),
});
export const sharedConsentsSchema = z.object({
  consents: z.array(z.object({
    id: z.string(), connection_id: z.string(), catalog_service_id: z.string(),
    connection_name: z.string().nullable().optional(),
    operation: z.string(), conversation_id: z.string(), decision: z.enum(["pending", "always", "once"]),
    expires_at: z.string().nullable(), scope: z.string(),
  })),
});
export type SharingForm = z.infer<typeof sharingFormSchema>;
export type Sharing = z.infer<typeof sharingSchema>;
export type SharedNotes = z.infer<typeof sharedNotesSchema>;
