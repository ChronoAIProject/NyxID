import { z } from "zod";

export const ASSISTANT_RECEIPT_VERSION = 1 as const;

const persistedReceiptSchema = z.object({
  placeholderId: z.string().min(1),
  conversationId: z.string().min(1).optional(),
  stateVersion: z.number().int().positive().safe().optional(),
  createdAt: z.number(),
  updatedAt: z.number(),
});

const persistedDeletionIntentSchema = z.object({
  placeholderId: z.string().min(1),
  conversationId: z.string().min(1).optional(),
  createdAt: z.number(),
});

const persistedAssistantReceiptsEnvelopeSchema = z.object({
  version: z.literal(ASSISTANT_RECEIPT_VERSION),
  ownerUserId: z.string().min(1).nullable(),
  receipts: z.record(z.string(), z.unknown()),
  deletionIntents: z.record(z.string(), z.unknown()),
});

export type AssistantCreateReceipt = z.infer<typeof persistedReceiptSchema>;
export type AssistantDeletionIntent = z.infer<
  typeof persistedDeletionIntentSchema
>;

export interface AssistantReceiptState {
  readonly version: typeof ASSISTANT_RECEIPT_VERSION;
  readonly ownerUserId: string | null;
  readonly receipts: Record<string, AssistantCreateReceipt>;
  readonly deletionIntents: Record<string, AssistantDeletionIntent>;
}

const MAX_FUTURE_SKEW_MS = 5 * 60 * 1_000;

function normalizeTimestamp(value: number, now: number): number | undefined {
  if (!Number.isFinite(value) || value < 0) return undefined;
  return value > now + MAX_FUTURE_SKEW_MS ? now : value;
}

export function emptyAssistantReceiptState(
  ownerUserId: string | null,
): AssistantReceiptState {
  return {
    version: ASSISTANT_RECEIPT_VERSION,
    ownerUserId,
    receipts: {},
    deletionIntents: {},
  };
}

/** Parse the durable envelope while dropping only malformed child entries. */
export function parseAssistantReceiptState(
  value: unknown,
  now: number = Date.now(),
): AssistantReceiptState | undefined {
  const envelope = persistedAssistantReceiptsEnvelopeSchema.safeParse(value);
  if (!envelope.success) return undefined;

  const receipts: Record<string, AssistantCreateReceipt> = {};
  for (const [commandId, candidate] of Object.entries(envelope.data.receipts)) {
    const parsed = persistedReceiptSchema.safeParse(candidate);
    if (!parsed.success) continue;
    const createdAt = normalizeTimestamp(parsed.data.createdAt, now);
    const updatedAt = normalizeTimestamp(parsed.data.updatedAt, now);
    if (createdAt === undefined || updatedAt === undefined) continue;
    receipts[commandId] = { ...parsed.data, createdAt, updatedAt };
  }

  const deletionIntents: Record<string, AssistantDeletionIntent> = {};
  for (const [commandId, candidate] of Object.entries(
    envelope.data.deletionIntents,
  )) {
    const parsed = persistedDeletionIntentSchema.safeParse(candidate);
    if (!parsed.success) continue;
    const createdAt = normalizeTimestamp(parsed.data.createdAt, now);
    if (createdAt === undefined) continue;
    deletionIntents[commandId] = { ...parsed.data, createdAt };
  }

  return {
    version: ASSISTANT_RECEIPT_VERSION,
    ownerUserId: envelope.data.ownerUserId,
    receipts,
    deletionIntents,
  };
}
