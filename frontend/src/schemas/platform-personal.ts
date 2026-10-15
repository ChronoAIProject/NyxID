import { z } from "zod";

const date = z.iso.datetime({ offset: true });
const version = z.number().int().positive().max(Number.MAX_SAFE_INTEGER);
const manager = z.object({
  id: z.string(),
  username: z.string().nullable(),
  display_name: z.string(),
});
const sender = z.object({
  id: z.string().regex(/^-?\d+$/),
  display_name: z.string().nullable(),
});
export const personalLinkPreviewSchema = z.object({
  purpose: z.literal("personal_nyxbot"),
  status: z.literal("pending"),
  manager,
  sender,
  expires_at: date,
  invitation_required: z.boolean(),
  requires_same_chat_confirmation: z.literal(true),
});
export const personalLinkReviewSchema = z.object({
  challenge_id: z.uuid(),
  status: z.literal("review"),
  expires_at: date,
  review_digest: z.string().min(1),
  account: z.object({ id: z.string(), display_name: z.string().nullable() }),
  permission_mode: z.enum(["nyxmode", "ask_every_time", "autopilot"]),
  billing: z.literal("personal"),
  replaces_existing_binding: z.boolean(),
  disclosures: z.object({
    telegram_transport: z.literal(true),
    existing_nyxbot_access: z.literal(true),
    can_approve_sensitive_actions: z.literal(true),
    started_work_may_continue_after_unlink: z.literal(true),
  }),
});
export const personalLinkStatusSchema = z.object({
  challenge_id: z.uuid(),
  status: z.enum([
    "review",
    "awaiting_chat_confirmation",
    "confirmed",
    "denied",
    "cancelled",
    "expired",
  ]),
  expires_at: date,
  binding: z.object({ id: z.uuid(), generation: version }).nullable(),
  confirmation_command: z
    .string()
    .regex(/^\/confirm P-[A-Z0-9]{4}-[A-Z0-9]{4}$/)
    .nullable(),
});
export const personalBindingSchema = z.object({
  id: z.uuid(),
  generation: version,
  manager,
  sender,
  state: z.enum(["active", "revoked"]),
  conversation_id: z.string().nullable(),
  consented_at: date,
  revoked_at: date.nullable(),
});
export const personalBindingsSchema = z.object({
  bindings: z.array(personalBindingSchema),
  next_before_id: z.uuid().nullable(),
});
export type PersonalBinding = z.infer<typeof personalBindingSchema>;
export type PersonalLinkReview = z.infer<typeof personalLinkReviewSchema>;

export const platformThreadSchema = z.object({
  platform: z.literal("telegram"),
  manager_id: z.string(),
  binding_id: z.uuid(),
  binding_generation: version,
});
export const platformTurnOriginSchema = z.object({
  binding_id: z.uuid(),
  binding_generation: version,
  manager_authority_epoch: version,
  personal_control_revision: version,
});
export type PlatformThread = z.infer<typeof platformThreadSchema>;
