import { z } from "zod";

export const cmaAgentIdSchema = z
  .string()
  .regex(/^agt_[A-Za-z0-9_-]{1,128}$/, "Invalid CMA session ID.");

export const cmaConnectionSchema = z.object({
  id: z.string().uuid(),
  name: z.string(),
  slug: z.string(),
  connected: z.boolean(),
  requires_connection: z.boolean(),
});
export type CmaConnection = z.infer<typeof cmaConnectionSchema>;

export const cmaDiscoverySchema = z.object({
  services: z.array(cmaConnectionSchema),
  custom_services: z.array(cmaConnectionSchema).default([]),
  total: z.number().int().nonnegative(),
  page: z.number().int().positive(),
  per_page: z.number().int().positive(),
});

export const cmaAgentSchema = z.object({
  id: cmaAgentIdSchema,
  title: z.string().nullish(),
  owner_title: z.string().nullish(),
  status: z.string().max(80),
});
export type CmaAgent = z.infer<typeof cmaAgentSchema>;

export const cmaAgentPageSchema = z.object({
  items: z.array(cmaAgentSchema),
  next: z.string().nullish(),
});

export const cmaRoutesSchema = z.object({
  agents: z.string(),
  agent: z.string(),
  head: z.string(),
  events: z.string(),
});
export type CmaRoutes = z.infer<typeof cmaRoutesSchema>;

export const cmaSessionReferenceSchema = z.object({
  connection: cmaConnectionSchema,
  agentId: cmaAgentIdSchema,
  title: z.string().max(300),
  routes: cmaRoutesSchema,
});
export type CmaSessionReference = z.infer<typeof cmaSessionReferenceSchema>;

export const cmaSessionKeySchema = z
  .string()
  .regex(/^[0-9a-f-]{36}:agt_[A-Za-z0-9_-]{1,128}$/i);
export const cmaSessionPreferencesSchema = z.record(
  cmaSessionKeySchema,
  z.boolean(),
);

const eventKind = z.object({ type: z.string() }).passthrough();
export const cmaEventItemSchema = z.object({
  cursor: z.string().min(1).max(1024),
  sequence: z.number().int().nonnegative().safe(),
  appended_at: z.string(),
  kind: eventKind.nullable(),
});
export type CmaEventItem = z.infer<typeof cmaEventItemSchema>;

export const cmaEventHeadSchema = z.object({
  object: z.literal("cma.agent.event_head"),
  schema_version: z.literal(1),
  agent_id: cmaAgentIdSchema,
  incarnation: z.string().min(1).max(1024),
  through_cursor: z.string().min(1).max(1024),
  active_turn: cmaEventItemSchema.nullable(),
});

export const cmaDurableEventSchema = cmaEventItemSchema.extend({
  agent_id: cmaAgentIdSchema,
  incarnation: z.string(),
});

export type CmaSessionStatus =
  | "starting"
  | "working"
  | "ready"
  | "completed"
  | "waiting"
  | "paused"
  | "failed"
  | "stopped"
  | "deleted"
  | "unknown";
export type CmaStreamStatus =
  | "connecting"
  | "live"
  | "reconnecting"
  | "unavailable";
export interface CmaSessionAction {
  readonly id: string;
  readonly title: string;
  readonly status: "running" | "completed" | "failed" | "unknown";
  readonly at: string;
}
export interface CmaSessionState {
  readonly connection: CmaStreamStatus;
  readonly status: CmaSessionStatus;
  readonly title: string;
  readonly actions: readonly CmaSessionAction[];
  readonly updatedAt?: string;
  readonly notice?: string;
  readonly error?: string;
}
