import { z } from "zod";

export const GUEST_SKILL_AUDIENCES = ["private", "shared"] as const;

const pin = z.object({
  source: z.literal("ornn"),
  skill_id: z.uuid(),
  name: z.string(),
  version: z.string().min(1),
  sha256: z.string().regex(/^[0-9a-f]{64}$/),
});

const attachment = z.object({
  skill_id: z.uuid(),
  audiences: z.array(z.enum(GUEST_SKILL_AUDIENCES)).max(2),
  all_requests_approved: z.boolean(),
});

export const guestSkillInventorySchema = z.object({
  revision: z.number().int().nonnegative(),
  skills_revision: z.number().int().nonnegative(),
  available: z.array(pin.extend({ dependencies: z.array(pin).max(16) })).max(16),
  attachments: z.array(attachment).max(16),
});

export type GuestSkillAttachment = Readonly<z.infer<typeof attachment>>;
export type GuestSkillInventory = Readonly<z.infer<typeof guestSkillInventorySchema>>;

export interface GuestSkillSelection {
  readonly expected_revision: number;
  readonly skills_revision: number;
  readonly attachments: readonly GuestSkillAttachment[];
}
