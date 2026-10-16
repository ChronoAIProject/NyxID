import { z } from "zod";
import { requestJson } from "./http";
import {
  agentCreateSchema,
  agentDetailSchema,
  agentSchema,
  type AgentCreate,
  type AgentPatch,
} from "../../features/agents/agentContracts";

const ROOT = "/assistant/nyxagent";
const agentPath = (id: string) => `${ROOT}/agents/${encodeURIComponent(id)}`;
export const agentsApi = {
  async list(signal?: AbortSignal) {
    return z
      .object({
        agents: z.array(agentSchema),
        limits: z.object({
          max_live_subagents: z.number(),
          max_live_subagents_limit: z.number(),
        }),
      })
      .parse(
        await requestJson(`${ROOT}/agents?include_destroyed=true`, { signal }),
      );
  },
  async detail(id: string, signal?: AbortSignal) {
    const detail = agentDetailSchema.parse(
      await requestJson(agentPath(id), { signal }),
    );
    if (detail.agent.id !== id)
      throw new Error("The server returned a different agent. Please reload.");
    return detail;
  },
  async create(input: AgentCreate) {
    const { display_name, persona, ...body } = agentCreateSchema.parse(input);
    return z
      .object({
        id: z.string(),
        name: z.string(),
        home_conversation_id: z.string(),
      })
      .parse(
        await requestJson(`${ROOT}/agents`, {
          method: "POST",
          body: {
            ...body,
            ...(display_name ? { display_name } : {}),
            ...(persona ? { persona } : {}),
          },
        }),
      );
  },
  async update(id: string, body: AgentPatch) {
    await requestJson(agentPath(id), { method: "PATCH", body });
  },
  async grants(
    id: string,
    body: {
      services: string[];
      account_read: boolean;
      guest_access?: Record<string, string>;
    },
  ) {
    // Omit machine/login fields: the server preserves those grants.
    await requestJson(`${agentPath(id)}/grants`, { method: "PUT", body });
  },
  async destroy(id: string) {
    await requestJson(`${agentPath(id)}/destroy`, { method: "POST" });
  },
  async remove(id: string) {
    await requestJson(agentPath(id), { method: "DELETE" });
  },
  async models(signal?: AbortSignal) {
    return z
      .array(
        z.object({
          id: z.string(),
          label: z.string(),
          available: z.boolean(),
          description: z.string().nullish(),
        }),
      )
      .parse(await requestJson(`${ROOT}/models`, { signal }));
  },
  async remember(id: string, text: string) {
    await requestJson(`${agentPath(id)}/memory`, {
      method: "POST",
      body: { text },
    });
  },
  async forget(id: string, noteId: string) {
    await requestJson(`${agentPath(id)}/memory/${encodeURIComponent(noteId)}`, {
      method: "DELETE",
    });
  },
};
