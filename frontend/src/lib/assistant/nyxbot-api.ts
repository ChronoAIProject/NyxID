import { assistantJson } from "@/lib/assistant/assistant-http";
import {
  assistantAgentCreatedSchema,
  assistantAgentDestroyedSchema,
  assistantAgentDetailSchema,
  assistantAgentListSchema,
  nyxAgentChannelConnectSchema,
  nyxAgentChannelLinkedSchema,
  nyxAgentChannelListSchema,
  nyxAgentSettingsSchema,
  type AssistantAgentCreate,
  type AssistantAgentGrants,
  type NyxAgentSettingsUpdate,
} from "@/schemas/assistant-nyxagent";

const ROOT = "/assistant/nyxagent";
const agentPath = (id: string) => `${ROOT}/agents/${encodeURIComponent(id)}`;

/** NyxBot agents, settings and channel-bot endpoints. The server owns all state. */
export const nyxBotApi = {
  /** NyxBot first, then specialists; NyxBot is created on first use. */
  async agents(includeDestroyed = true) {
    return assistantAgentListSchema.parse(
      await assistantJson(`${ROOT}/agents?include_destroyed=${String(includeDestroyed)}`),
    );
  },
  async agent(id: string) {
    return assistantAgentDetailSchema.parse(await assistantJson(agentPath(id)));
  },
  async createAgent(body: AssistantAgentCreate) {
    return assistantAgentCreatedSchema.parse(
      await assistantJson(`${ROOT}/agents`, { method: "POST", body }),
    );
  },
  async updateAgent(id: string, body: { name?: string; description?: string }) {
    await assistantJson(agentPath(id), { method: "PATCH", body });
  },
  /** Replaces a specialist's grants. */
  async setGrants(id: string, body: AssistantAgentGrants) {
    await assistantJson(`${agentPath(id)}/grants`, { method: "PUT", body });
  },
  async destroyAgent(id: string) {
    return assistantAgentDestroyedSchema.parse(
      await assistantJson(`${agentPath(id)}/destroy`, { method: "POST" }),
    );
  },
  /** Only for destroyed specialists: removes the agent and all its threads. */
  async deleteAgent(id: string) {
    await assistantJson(agentPath(id), { method: "DELETE" });
  },
  async forget(agentId: string, noteId: string) {
    await assistantJson(`${agentPath(agentId)}/memory/${encodeURIComponent(noteId)}`, {
      method: "DELETE",
    });
  },
  async settings() {
    return nyxAgentSettingsSchema.parse(await assistantJson(`${ROOT}/settings`));
  },
  async updateSettings(update: NyxAgentSettingsUpdate) {
    return nyxAgentSettingsSchema.parse(
      await assistantJson(`${ROOT}/settings`, { method: "PUT", body: update }),
    );
  },
  async channels() {
    return nyxAgentChannelListSchema.parse(await assistantJson(`${ROOT}/channels`))
      .channel_agents;
  },
  /**
   * Connects a channel bot to an agent (NyxBot by default), or issues a fresh
   * owner link and relinks when it is already connected.
   */
  async connectChannel(botId: string, agentId?: string) {
    return nyxAgentChannelConnectSchema.parse(
      await assistantJson(`${ROOT}/channels`, {
        method: "POST",
        body: { bot_id: botId, ...(agentId ? { agent_id: agentId } : {}) },
      }),
    );
  },
  async linkChannel(channelAgentId: string, agentId: string) {
    return nyxAgentChannelLinkedSchema.parse(
      await assistantJson(`${ROOT}/channels/${encodeURIComponent(channelAgentId)}`, {
        method: "PATCH",
        body: { agent_id: agentId },
      }),
    );
  },
  async disconnectChannel(channelAgentId: string) {
    await assistantJson(`${ROOT}/channels/${encodeURIComponent(channelAgentId)}`, {
      method: "DELETE",
    });
  },
};
