import { assistantJson } from "@/lib/assistant/assistant-http";
import {
  assistantAgentCreatedSchema,
  assistantAgentDestroyedSchema,
  assistantAgentDetailSchema,
  assistantAgentListSchema,
  assistantGroupListSchema,
  assistantGroupMessagesSchema,
  assistantGroupPostedSchema,
  assistantGroupSchema,
  nyxAgentChannelChatListSchema,
  nyxAgentChannelChatUpdatedSchema,
  nyxAgentChannelConnectSchema,
  nyxAgentChannelLinkedSchema,
  nyxAgentChannelListSchema,
  nyxAgentSettingsSchema,
  type AssistantAgentCreate,
  type AssistantAgentGrantsRequest,
  type AssistantGroupForm,
  type AssistantGroupUpdate,
  type NyxAgentChannelChatSettings,
  type NyxAgentSettingsUpdate,
} from "@/schemas/assistant-nyxagent";

const ROOT = "/assistant/nyxagent";
const agentPath = (id: string) => `${ROOT}/agents/${encodeURIComponent(id)}`;
const groupPath = (id: string) => `${ROOT}/groups/${encodeURIComponent(id)}`;

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
  /** Blank display name and persona are left out (the agent has none). */
  async createAgent({ display_name, persona, ...body }: AssistantAgentCreate) {
    return assistantAgentCreatedSchema.parse(
      await assistantJson(`${ROOT}/agents`, {
        method: "POST",
        body: {
          ...body,
          ...(display_name ? { display_name } : {}),
          ...(persona ? { persona } : {}),
        },
      }),
    );
  },
  /** An empty `display_name` or `persona` clears it. */
  async updateAgent(
    id: string,
    body: { name?: string; description?: string; display_name?: string; persona?: string },
  ) {
    await assistantJson(agentPath(id), { method: "PATCH", body });
  },
  /** Replaces a specialist's grants. */
  async setGrants(id: string, body: AssistantAgentGrantsRequest) {
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
  async remember(agentId: string, text: string, replaceId?: string) {
    await assistantJson(`${agentPath(agentId)}/memory`, {
      method: "POST",
      body: { text, ...(replaceId ? { replace_id: replaceId } : {}) },
    });
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
  /** Who may talk to the agent in the bot's private chats. */
  async setPrivateChats(channelAgentId: string, privateChats: "owner" | "everyone") {
    await assistantJson(`${ROOT}/channels/${encodeURIComponent(channelAgentId)}`, {
      method: "PATCH",
      body: { private_chats: privateChats },
    });
  },
  /** The bot's chats, most recent first. */
  async channelChats(channelAgentId: string) {
    return nyxAgentChannelChatListSchema.parse(
      await assistantJson(`${ROOT}/channels/${encodeURIComponent(channelAgentId)}/chats`),
    ).chats;
  },
  async updateChannelChat(
    channelAgentId: string,
    chatId: string,
    settings: NyxAgentChannelChatSettings,
  ) {
    return nyxAgentChannelChatUpdatedSchema.parse(
      await assistantJson(
        `${ROOT}/channels/${encodeURIComponent(channelAgentId)}/chats/${encodeURIComponent(chatId)}`,
        { method: "PATCH", body: settings },
      ),
    );
  },
  async disconnectChannel(channelAgentId: string) {
    await assistantJson(`${ROOT}/channels/${encodeURIComponent(channelAgentId)}`, {
      method: "DELETE",
    });
  },
  /** Newest activity first. */
  async groups() {
    return assistantGroupListSchema.parse(await assistantJson(`${ROOT}/groups`)).groups;
  },
  async group(id: string) {
    return assistantGroupSchema.parse(await assistantJson(groupPath(id)));
  },
  async createGroup(body: AssistantGroupForm) {
    return assistantGroupSchema.parse(
      await assistantJson(`${ROOT}/groups`, { method: "POST", body }),
    );
  },
  async updateGroup(id: string, body: AssistantGroupUpdate) {
    return assistantGroupSchema.parse(
      await assistantJson(groupPath(id), { method: "PATCH", body }),
    );
  },
  async deleteGroup(id: string) {
    await assistantJson(groupPath(id), { method: "DELETE" });
  },
  /** The newest page, or the page before `beforeSeq`; messages ascend by seq. */
  async groupMessages(id: string, beforeSeq?: number) {
    const query = beforeSeq ? `&before_seq=${String(beforeSeq)}` : "";
    return assistantGroupMessagesSchema.parse(
      await assistantJson(`${groupPath(id)}/messages?limit=50${query}`),
    );
  },
  /** Accepted (202): replies arrive later as new messages. */
  async postGroupMessage(id: string, text: string, attachmentIds?: string[]) {
    return assistantGroupPostedSchema.parse(
      await assistantJson(`${groupPath(id)}/messages`, { method: "POST", body: { text, ...(attachmentIds?.length ? { attachment_ids: attachmentIds } : {}) } }),
    );
  },
};
