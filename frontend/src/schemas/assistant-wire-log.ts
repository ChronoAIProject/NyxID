import { z } from "zod";

export const assistantUpstreamOutcomeSchema = z.enum([
  "response",
  "no_response",
  "unknown",
]);

export const assistantUpstreamIdentitySchema = z
  .object({
    mode: z.string(),
    forward_access_token: z.boolean(),
    inject_delegation_token: z.boolean(),
    bridge_minted: z.boolean(),
  })
  .strict();

export const assistantUpstreamResponseHeaderSchema = z
  .object({
    value: z.string(),
    truncated: z.boolean(),
  })
  .strict();

export const assistantUpstreamResponseSchema = z
  .object({
    status: z.number().int().min(100).max(599),
    headers: z.record(z.string(), assistantUpstreamResponseHeaderSchema),
    sse: z.boolean(),
  })
  .strict();

export const assistantFullUpstreamEnvelopeSchema = z
  .object({
    degraded: z.literal(false),
    method: z.string(),
    path: z.string(),
    commandType: z.string().nullable(),
    body: z.unknown(),
    headers: z.record(z.string(), z.string()),
    identity: assistantUpstreamIdentitySchema,
    truncated: z.boolean(),
    response: assistantUpstreamResponseSchema.nullable().optional(),
    upstreamOutcome: assistantUpstreamOutcomeSchema.optional(),
    droppedBody: z.boolean().optional(),
    droppedHeaders: z.boolean().optional(),
  })
  .strict();

export const assistantMinimalUpstreamEnvelopeSchema = z
  .object({
    degraded: z.literal(true),
    method: z.string(),
    path: z.string(),
    commandType: z.string().optional(),
    upstreamOutcome: assistantUpstreamOutcomeSchema.optional(),
    status: z.number().int().min(100).max(599).optional(),
  })
  .strict();

export const assistantUpstreamEnvelopeSchema = z.discriminatedUnion(
  "degraded",
  [assistantFullUpstreamEnvelopeSchema, assistantMinimalUpstreamEnvelopeSchema],
);

export const assistantUpstreamEnvelopeHeaderSchema = z
  .object({
    version: z.literal(2),
    echoes: z.array(assistantUpstreamEnvelopeSchema).min(1),
    droppedEchoCount: z.number().int().nonnegative(),
  })
  .strict();

const legacyAssistantUpstreamEnvelopeSchema = z.object({
  method: z.string(),
  path: z.string(),
  commandType: z.string().nullable(),
  body: z.unknown(),
  headers: z.record(z.string(), z.string()),
  identity: assistantUpstreamIdentitySchema.strip(),
  truncated: z.boolean(),
});

export const assistantUpstreamEnvelopeHeaderDecoderSchema = z
  .union([
    assistantUpstreamEnvelopeHeaderSchema,
    z.array(legacyAssistantUpstreamEnvelopeSchema).min(1),
  ])
  .transform((value) => {
    if (!Array.isArray(value)) return value;
    return {
      version: 2 as const,
      echoes: value.map((echo) => ({ ...echo, degraded: false as const })),
      droppedEchoCount: 0,
    };
  });

export const assistantWireLineSchema = z
  .object({
    text: z.string(),
    ending: z.enum(["\n", "\r\n", "\r", ""]),
  })
  .strict();

export const assistantWireCaptureOutcomeSchema = z.enum([
  "complete",
  "cancelled",
  "network_error",
  "worker_error",
  "protocol_cancel",
]);

export const assistantTransportOutcomeSchema = z
  .string()
  .trim()
  .min(1)
  .max(128)
  .regex(/^[A-Za-z0-9._:-]+$/);

const assistantTransportTelemetryShape = {
  transportOutcome: assistantTransportOutcomeSchema.optional(),
  framesSeen: z.number().int().nonnegative().optional(),
  printableFramesSeen: z.number().int().nonnegative().optional(),
  printableTurnEvents: z.number().int().nonnegative().optional(),
  wireBytes: z.number().int().nonnegative().optional(),
  terminalReceived: z.boolean().optional(),
  firstFrameMs: z.number().finite().nonnegative().nullable().optional(),
  lastFrameMs: z.number().finite().nonnegative().nullable().optional(),
} as const;

const assistantWireSseCaptureSchema = z
  .object({
    lines: z.array(assistantWireLineSchema),
    bytes: z.number().int().nonnegative(),
    retainedBytes: z.number().int().nonnegative(),
    truncated: z.boolean(),
  })
  .strict();

const assistantWireBodyCaptureSchema = z
  .object({
    text: z.string(),
    bytes: z.number().int().nonnegative(),
    truncated: z.boolean(),
  })
  .strict();

const assistantWireOpenCaptureSchema = z
  .object({
    state: z.literal("open"),
    sse: assistantWireSseCaptureSchema.optional(),
    body: assistantWireBodyCaptureSchema.optional(),
    ...assistantTransportTelemetryShape,
  })
  .strict();

const assistantWireSettledCaptureSchema = z
  .object({
    state: z.literal("settled"),
    /** Compatibility alias for consumers predating transport telemetry. */
    outcome: assistantWireCaptureOutcomeSchema,
    wireOutcome: assistantWireCaptureOutcomeSchema,
    sse: assistantWireSseCaptureSchema.optional(),
    body: assistantWireBodyCaptureSchema.optional(),
    ...assistantTransportTelemetryShape,
  })
  .strict();

export const assistantWireCaptureSchema = z.discriminatedUnion("state", [
  z.object({ state: z.literal("evicted") }).strict(),
  assistantWireOpenCaptureSchema,
  assistantWireSettledCaptureSchema,
]);

const assistantWireExchangeBaseSchema = z
  .object({
    id: z.string(),
    ts: z.number().finite(),
    kind: z.enum(["sse", "header"]),
    status: z.number().int().min(100).max(599),
    upstreamEchoes: z.array(assistantUpstreamEnvelopeSchema).min(1),
    droppedEchoCount: z.number().int().nonnegative().optional(),
  })
  .strict();

export const assistantWireLogExchangeSchema = assistantWireExchangeBaseSchema
  .extend({ capture: assistantWireCaptureSchema.optional() })
  .strict();

export const persistedAssistantWireLogExchangeSchema =
  assistantWireExchangeBaseSchema;

export const assistantWireLogPersistedSchema = z
  .object({
    captureEnabled: z.boolean(),
    showResponses: z.boolean(),
    entries: z.array(persistedAssistantWireLogExchangeSchema),
  })
  .strict();

export type AssistantUpstreamEnvelope = z.infer<
  typeof assistantUpstreamEnvelopeSchema
>;
export type AssistantUpstreamEnvelopeHeader = z.infer<
  typeof assistantUpstreamEnvelopeHeaderSchema
>;
export type AssistantWireLine = z.infer<typeof assistantWireLineSchema>;
export type AssistantWireCaptureOutcome = z.infer<
  typeof assistantWireCaptureOutcomeSchema
>;
export type AssistantTransportOutcome = z.infer<
  typeof assistantTransportOutcomeSchema
>;
export type AssistantWireCapture = z.infer<typeof assistantWireCaptureSchema>;
export type AssistantWireLogExchange = z.infer<
  typeof assistantWireLogExchangeSchema
>;
export type PersistedAssistantWireLogExchange = Omit<
  AssistantWireLogExchange,
  "capture"
>;
