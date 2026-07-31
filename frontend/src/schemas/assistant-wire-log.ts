import { z } from "zod";

export const assistantUpstreamIdentitySchema = z
  .object({
    mode: z.string(),
    forward_access_token: z.boolean(),
    inject_delegation_token: z.boolean(),
    bridge_minted: z.boolean(),
  })
  .strict();

export const assistantUpstreamEnvelopeSchema = z
  .object({
    method: z.string(),
    path: z.string(),
    commandType: z.string().nullable(),
    body: z.unknown(),
    headers: z.record(z.string(), z.string()),
    identity: assistantUpstreamIdentitySchema,
    truncated: z.boolean(),
  })
  .strict();

export const assistantUpstreamEnvelopeListSchema = z.array(
  assistantUpstreamEnvelopeSchema,
);

export const assistantWireLogEntrySchema = assistantUpstreamEnvelopeSchema
  .extend({
    id: z.string(),
    ts: z.number().finite(),
    kind: z.enum(["sse", "header"]),
    status: z.number().int().min(100).max(599),
  })
  .strict();

export const assistantWireLogPersistedSchema = z
  .object({
    captureEnabled: z.boolean(),
    entries: z.array(assistantWireLogEntrySchema),
  })
  .strict();

export type AssistantUpstreamEnvelope = z.infer<
  typeof assistantUpstreamEnvelopeSchema
>;
export type AssistantWireLogEntry = z.infer<
  typeof assistantWireLogEntrySchema
>;
