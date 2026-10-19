import { z } from "zod";

const date = z.iso.datetime({ offset: true });
const version = z.number().int().positive().max(Number.MAX_SAFE_INTEGER);
export const legacyTelegramChat = {
  platform: "telegram" as const,
  display_name: "Telegram",
  login_command: "/login",
  logout_command: "/logout",
  status_command: "/status",
  help_command: "/help",
  question_instruction: "Send ordinary text in this bot's private chat.",
  transport_disclosure:
    "This Telegram bot is operated by the platform. Telegram bot chats are not end-to-end encrypted.",
};
export const personalChatSchema = z
  .object({
    platform: z.enum([
      "telegram",
      "slack",
      "discord",
      "whatsapp",
      "lark",
      "feishu",
      "x",
    ]),
    display_name: z.string().min(1).max(32),
    login_command: z.string().min(1).max(32),
    logout_command: z.string().min(1).max(32),
    status_command: z.string().min(1).max(32),
    help_command: z.string().min(1).max(32),
    question_instruction: z.string().min(1).max(512),
    transport_disclosure: z.string().min(1).max(512),
  })
  .superRefine((chat, context) => {
    const prefix =
      chat.platform === "telegram"
        ? "/"
        : chat.platform === "discord"
          ? "/nyx "
          : "";
    for (const [field, verb] of [
      ["login_command", "login"],
      ["logout_command", "logout"],
      ["status_command", "status"],
      ["help_command", "help"],
    ] as const) {
      if (chat[field] !== `${prefix}${verb}`)
        context.addIssue({
          code: "custom",
          path: [field],
          message: "Unsupported chat control surface",
        });
    }
  });
const chat = personalChatSchema.default(legacyTelegramChat);
const opaqueIdentity = z
  .string()
  .min(1)
  .refine((value) => new TextEncoder().encode(value).byteLength <= 256)
  .refine(
    (value) => value.trim() === value &&
      [...value].every((character) => {
        const point = character.codePointAt(0)!;
        return point >= 0x20 && !(point >= 0x7f && point <= 0x9f);
      }),
  );
const manager = z.object({
  id: z.string(),
  username: z.string().nullable(),
  display_name: z.string(),
});
const sender = z.object({
  id: opaqueIdentity,
  display_name: z.string().nullable(),
});
export const personalLinkPreviewSchema = z
  .object({
    chat,
    purpose: z.literal("personal_nyxbot"),
    status: z.literal("pending"),
    manager,
    sender,
    expires_at: date,
    invitation_required: z.boolean(),
    requires_same_chat_confirmation: z.literal(true),
  })
  .superRefine((response, context) => {
    if (
      response.chat.platform === "telegram" &&
      !/^-?\d+$/.test(response.sender.id)
    )
      context.addIssue({
        code: "custom",
        path: ["sender", "id"],
        message: "Invalid Telegram identity",
      });
  });
export const personalLinkReviewSchema = z.object({
  chat,
  challenge_id: z.uuid(),
  status: z.literal("review"),
  expires_at: date,
  review_digest: z.string().min(1),
  account: z.object({ id: z.string(), display_name: z.string().nullable() }),
  permission_mode: z.enum(["nyxmode", "ask_every_time", "autopilot"]),
  billing: z.literal("personal"),
  replaces_existing_binding: z.boolean(),
  disclosures: z.object({
    telegram_transport: z.boolean(),
    existing_nyxbot_access: z.literal(true),
    can_approve_sensitive_actions: z.literal(true),
    started_work_may_continue_after_unlink: z.literal(true),
  }),
});
export const personalLinkStatusSchema = z
  .object({
    chat,
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
      .regex(
        /^(?:\/confirm |confirm |\/nyx confirm code:)P-[A-Z0-9]{4}-[A-Z0-9]{4}$/,
      )
      .nullable(),
  })
  .superRefine((response, context) => {
    if (response.confirmation_command === null) return;
    const prefix =
      response.chat.platform === "telegram"
        ? "/confirm "
        : response.chat.platform === "discord"
          ? "/nyx confirm code:"
          : "confirm ";
    if (!response.confirmation_command.startsWith(prefix))
      context.addIssue({
        code: "custom",
        path: ["confirmation_command"],
        message: "Confirmation does not match this chat platform",
      });
  });
export const personalBindingSchema = z.object({
  chat,
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
  platform: z.enum([
    "telegram",
    "slack",
    "discord",
    "whatsapp",
    "lark",
    "feishu",
    "x",
  ]),
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
