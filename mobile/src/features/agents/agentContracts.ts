import { z } from "zod";
import { conversationIdSchema } from "../assistant/assistantTypes";
import type { ServiceConnection } from "../services/serviceContracts";

export const accessModeSchema = z.enum([
  "autopilot",
  "nyxmode",
  "ask_every_time",
]);
const guestAccessSchema = z.enum(["read", "use", "all"]);
export const agentSchema = z.object({
  id: z.string(),
  kind: z.enum(["nyxbot", "specialist"]),
  name: z.string(),
  display_name: z.string().nullish(),
  persona: z.string().nullish(),
  description: z.string(),
  model: z.string().optional(),
  access_mode_override: accessModeSchema.nullish(),
  status: z.enum(["running", "idle", "destroyed"]),
  owner_id: z.string().optional(),
  owner_name: z.string().nullish(),
  owner_kind: z.enum(["person", "org"]).optional(),
  org_role: z.enum(["owner", "admin", "member", "viewer"]).nullish(),
  can_maintain: z.boolean().optional(),
  can_use: z.boolean().optional(),
  services: z.array(z.string()).default([]),
  account_read: z.boolean().default(false),
  guest_access: z.record(z.string(), guestAccessSchema).default({}),
  home_conversation_id: z.string().nullish(),
  memory_count: z.number().default(0),
  pending_acknowledgements: z.number().default(0),
  pending_requests: z
    .array(
      z.object({
        request_id: z.string(),
        conversation_id: conversationIdSchema,
        summary: z.string(),
        expires_at: z.string().optional(),
      }),
    )
    .default([]),
  last_reply: z
    .object({
      seq: z.number(),
      status: z.string(),
      text: z.string(),
      created_at: z.string(),
    })
    .nullish(),
  last_active_at: z.string().optional(),
  channels: z
    .array(
      z.object({
        id: z.string(),
        platform: z.string(),
        bot_label: z.string(),
        status: z.string(),
      }),
    )
    .default([]),
});
export type AssistantAgent = z.infer<typeof agentSchema>;
export const agentDetailSchema = z.object({
  agent: agentSchema,
  memory: z
    .array(
      z.object({
        id: z.string(),
        text: z.string(),
        created_at: z.string(),
        updated_at: z.string(),
      }),
    )
    .default([]),
  threads: z
    .array(
      z.object({
        id: z.string(),
        title: z.string(),
        last_message_at: z.string(),
        running: z.boolean().default(false),
      }),
    )
    .default([]),
});

export const agentCreateSchema = z.object({
  name: z
    .string()
    .trim()
    .regex(
      /^[a-z0-9][a-z0-9-]{0,31}$/,
      "Use 1–32 lowercase letters, digits or hyphens, starting with a letter or digit.",
    ),
  display_name: z
    .string()
    .trim()
    .max(40, "Keep the display name under 40 characters."),
  description: z
    .string()
    .trim()
    .min(1, "Describe what the agent does.")
    .max(2048),
  persona: z.string().trim().max(2000),
  services: z.array(z.string()),
  account_read: z.boolean(),
});
export type AgentCreate = z.infer<typeof agentCreateSchema>;
export type AgentProfile = Pick<
  AssistantAgent,
  "name" | "description" | "access_mode_override"
> & {
  display_name: string;
  persona: string;
};
export type AgentPatch = Partial<AgentProfile>;

export function agentProfile(agent: AssistantAgent): AgentProfile {
  return {
    name: agent.name,
    description: agent.description,
    display_name: agent.display_name ?? "",
    persona: agent.persona ?? "",
    access_mode_override: agent.access_mode_override ?? null,
  };
}

export function buildAgentPatch(
  agent: AssistantAgent,
  values: AgentProfile,
): AgentPatch {
  const schema = agentCreateSchema
    .omit({ services: true, account_read: true })
    .extend({
      ...(agent.kind === "nyxbot"
        ? { name: z.string(), description: z.string().trim().max(2048) }
        : {}),
      access_mode_override: accessModeSchema.nullable().optional(),
    });
  const parsed = schema.parse(values);
  const baseline = agentProfile(agent);
  const patch: AgentPatch = {};
  for (const key of Object.keys(parsed) as (keyof AgentProfile)[]) {
    if (key === "name" && agent.kind === "nyxbot") continue;
    if (parsed[key] !== baseline[key])
      Object.assign(patch, { [key]: parsed[key] });
  }
  return patch;
}

export function profileHasConflict(
  baseline: AssistantAgent,
  current: AssistantAgent,
  patch: AgentPatch,
) {
  const old = agentProfile(baseline),
    live = agentProfile(current);
  return Object.keys(patch).some(
    (key) => old[key as keyof AgentProfile] !== live[key as keyof AgentProfile],
  );
}

export function grantSignature(
  agent: Pick<AssistantAgent, "services" | "account_read" | "guest_access">,
) {
  return JSON.stringify({
    services: [...agent.services].sort(),
    account_read: agent.account_read,
    guest_access: Object.entries(agent.guest_access).sort(([a], [b]) =>
      a.localeCompare(b),
    ),
  });
}

/** Org agents require exact UserService IDs. Personal agents accept service slugs. */
export function grantOptions(
  connections: ServiceConnection[],
  agent?: AssistantAgent,
) {
  const org = agent?.owner_kind === "org" ? agent.owner_id : undefined;
  const rows = connections.filter(
    (connection) =>
      connection.is_active &&
      (org
        ? connection.credential_source?.type === "org" &&
          connection.credential_source.org_id === org &&
          connection.credential_source.allowed
        : connection.credential_source?.type !== "org" ||
          connection.credential_source.allowed),
  );
  const seen = new Set<string>();
  return rows.flatMap((row) => {
    const value = org ? row.id : row.slug;
    if (seen.has(value)) return [];
    seen.add(value);
    return [
      {
        value,
        label: row.label,
        iconSlug: row.catalog_service_slug ?? row.slug,
        iconUrl: row.icon_url || row.catalog_icon_url,
      },
    ];
  });
}

export function changedGrants(
  baseline: AssistantAgent,
  services: string[],
  account_read: boolean,
  guests: Record<string, "read" | "use" | "all">,
) {
  const guest_access = Object.fromEntries(
    services
      .map((slug) => [slug, guests[slug] ?? "use"] as const)
      .filter(
        ([slug, level]) => level !== (baseline.guest_access[slug] ?? "use"),
      ),
  );
  return {
    services: [...new Set(services)],
    account_read,
    ...(Object.keys(guest_access).length ? { guest_access } : {}),
  };
}
