import { z } from "zod";

const requestIdSchema = z
  .string()
  .uuid()
  .regex(
    /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i,
  );
const conversationIdSchema = z.string().regex(/^nyxa-[0-9a-f]{32}$/);
const safeTextSchema = z.string().max(2 * 1024 * 1024);
const CHAT_RECOVERY_INITIAL_MS = 750;
const CHAT_RECOVERY_MAX_MS = 10_000;

export const NYXID_CHAT_ADMISSION_UNKNOWN_MESSAGE =
  "无法确认 NyxID 是否已收到这条消息，请不要重复发送。";

export const nyxIdChatRequestSchema = z
  .object({
    requestId: requestIdSchema,
    conversationId: conversationIdSchema.optional(),
    text: z.string().min(1).max(32_768),
  })
  .strict()
  .refine((request) => request.text.trim().length > 0, {
    path: ["text"],
    message: "Message must not be blank",
  });

const startedEventSchema = z
  .object({
    kind: z.literal("started"),
    requestId: requestIdSchema,
    conversationId: conversationIdSchema,
    turnId: z.string().min(1).max(128),
  })
  .strict();

const deltaEventSchema = z
  .object({
    kind: z.literal("delta"),
    requestId: requestIdSchema,
    text: safeTextSchema,
  })
  .strict();

const snapshotEventSchema = z
  .object({
    kind: z.literal("snapshot"),
    requestId: requestIdSchema,
    text: safeTextSchema,
  })
  .strict();

const completedEventSchema = z
  .object({
    kind: z.literal("completed"),
    requestId: requestIdSchema,
    conversationId: conversationIdSchema,
    status: z.enum(["completed", "failed", "cancelled"]),
    error: z
      .object({
        code: z.string().min(1).max(128),
        message: z.string().min(1).max(500),
      })
      .strict()
      .nullable(),
  })
  .strict();

export const nyxIdChatEventSchema = z.discriminatedUnion("kind", [
  startedEventSchema,
  deltaEventSchema,
  snapshotEventSchema,
  completedEventSchema,
]);

export const nyxIdChatCommandErrorSchema = z
  .object({
    kind: z.enum(["rejected", "admission_unknown"]),
    message: z.string().min(1).max(500),
  })
  .strict();

const historyMessageSchema = z
  .object({
    id: z.string().min(1).max(256),
    seq: z.number().int().positive(),
    turnId: z.string().min(1).max(128),
    role: z.enum(["user", "assistant", "orchestrator", "event"]),
    text: safeTextSchema,
    status: z.enum(["completed", "failed"]),
    errorCode: z.string().min(1).max(128).nullable(),
  })
  .strict();

export const nyxIdChatHistorySchema = z
  .object({
    conversation: z
      .object({
        id: conversationIdSchema,
        activeTurn: z.boolean(),
      })
      .strict(),
    messages: z.array(historyMessageSchema).max(100),
  })
  .strict();

export const nyxIdChatRecoverySchema = z
  .object({
    requestId: requestIdSchema,
    history: nyxIdChatHistorySchema,
  })
  .strict();

export type NyxIdChatRequest = z.infer<typeof nyxIdChatRequestSchema>;
export type NyxIdChatEvent = z.infer<typeof nyxIdChatEventSchema>;
export type NyxIdChatCompletedEvent = z.infer<typeof completedEventSchema>;
export type NyxIdChatHistory = z.infer<typeof nyxIdChatHistorySchema>;
export type NyxIdChatRecovery = z.infer<typeof nyxIdChatRecoverySchema>;

export class NyxIdChatSendError extends Error {
  readonly kind: "rejected" | "admission_unknown";

  constructor(kind: "rejected" | "admission_unknown", message: string) {
    super(message);
    this.name = "NyxIdChatSendError";
    this.kind = kind;
  }
}

export function parseNyxIdChatEvent(value: unknown): NyxIdChatEvent {
  return nyxIdChatEventSchema.parse(value);
}

export function nyxIdChatRecoveryDelayMs(
  attempt: number,
  random: () => number = Math.random,
): number {
  const normalizedAttempt =
    attempt === Number.POSITIVE_INFINITY
      ? 4
      : Number.isFinite(attempt) && attempt > 0
        ? Math.min(Math.floor(attempt), 4)
        : 0;
  if (normalizedAttempt === 0) return CHAT_RECOVERY_INITIAL_MS;

  let sample = 0;
  try {
    sample = random();
  } catch {
    sample = 0;
  }
  const normalizedSample = Number.isFinite(sample)
    ? Math.min(Math.max(sample, 0), 1)
    : 0;
  const ceiling = Math.min(
    CHAT_RECOVERY_MAX_MS,
    CHAT_RECOVERY_INITIAL_MS * 2 ** normalizedAttempt,
  );
  return Math.round(ceiling * (0.75 + normalizedSample * 0.25));
}
