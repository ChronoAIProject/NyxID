import { z } from "zod";

export const shareAudienceSchema = z.object({
  subject_kind: z.enum(["person", "org"]),
  subject_id: z.uuid(),
  role: z.enum(["manager", "user"]),
}).refine((s) => s.subject_kind !== "org" || s.role === "user", "Organizations receive user access");
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
  audience: z.array(shareAudienceSchema).default([]),
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
