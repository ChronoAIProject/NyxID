import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { nyxBotApi } from "@/lib/assistant/nyxbot-api";
import type {
  AssistantAgent,
  AssistantAgentCreate,
  AssistantAgentGrants,
  AssistantAgentList,
  NyxAgentChannelChatSettings,
  NyxAgentSettingsUpdate,
} from "@/schemas/assistant-nyxagent";
import { livePollInterval, useNyxAgentLiveConnected } from "@/hooks/use-nyxagent-live-status";
import { useAuthStore } from "@/stores/auth-store";

export const nyxBotQueryKeys = {
  root: (userId: string | undefined) => ["assistant", "nyxagent", userId] as const,
  /** Prefix of the agent list and every agent detail. */
  agents: (userId: string | undefined) => ["assistant", "nyxagent", userId, "agents"] as const,
  agent: (userId: string | undefined, agentId: string | undefined) =>
    ["assistant", "nyxagent", userId, "agents", "detail", agentId] as const,
  /** Prefix of every agent's thread list. */
  threads: (userId: string | undefined, agentId?: string) =>
    agentId === undefined
      ? (["assistant", "nyxagent", userId, "threads"] as const)
      : (["assistant", "nyxagent", userId, "threads", agentId] as const),
  settings: (userId: string | undefined) =>
    ["assistant", "nyxagent", userId, "settings"] as const,
  /** Prefix of the channel list and every channel's chats. */
  channels: (userId: string | undefined) =>
    ["assistant", "nyxagent", userId, "channels"] as const,
  channelChats: (userId: string | undefined, channelAgentId: string) =>
    ["assistant", "nyxagent", userId, "channels", channelAgentId, "chats"] as const,
  history: (userId: string | undefined, conversationId: string | null | undefined) =>
    ["assistant", "nyxagent", userId, "history", conversationId] as const,
};

const RUNNING_POLL_MS = 3000;

/** Agents are re-read while any of them is working, so statuses follow. */
export function agentsPollInterval(list: AssistantAgentList | undefined): number | false {
  return list?.agents.some((agent) => agent.status === "running") ? RUNNING_POLL_MS : false;
}

export function nyxBotOf(agents: readonly AssistantAgent[] | undefined) {
  return agents?.find((agent) => agent.kind === "nyxbot");
}

/** NyxBot and every specialist, destroyed ones included (the UI filters). */
export function useNyxBotAgents(enabled = true) {
  const userId = useAuthStore((state) => state.user?.id);
  const live = useNyxAgentLiveConnected();
  return useQuery({
    queryKey: nyxBotQueryKeys.agents(userId),
    queryFn: () => nyxBotApi.agents(true),
    enabled: enabled && Boolean(userId),
    retry: false,
    refetchInterval: (query) => livePollInterval(agentsPollInterval(query.state.data), live),
  });
}

/** One agent with its memory and threads. */
export function useNyxBotAgent(agentId: string | undefined, enabled = true) {
  const userId = useAuthStore((state) => state.user?.id);
  return useQuery({
    queryKey: nyxBotQueryKeys.agent(userId, agentId),
    queryFn: () => nyxBotApi.agent(agentId!),
    enabled: enabled && Boolean(userId && agentId),
    retry: false,
  });
}

function useAgentsMutation<TVariables, TResult>(
  mutationFn: (variables: TVariables) => Promise<TResult>,
  alsoChannels = false,
) {
  const userId = useAuthStore((state) => state.user?.id);
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn,
    onSettled: async () => {
      await Promise.all([
        queryClient.invalidateQueries({ queryKey: nyxBotQueryKeys.agents(userId) }),
        ...(alsoChannels
          ? [queryClient.invalidateQueries({ queryKey: nyxBotQueryKeys.channels(userId) })]
          : []),
      ]);
    },
  });
}

export function useCreateNyxBotAgent() {
  return useAgentsMutation((body: AssistantAgentCreate) => nyxBotApi.createAgent(body));
}

export function useUpdateNyxBotAgent() {
  return useAgentsMutation(
    ({
      id,
      ...body
    }: {
      id: string;
      name?: string;
      description?: string;
      display_name?: string;
      persona?: string;
    }) =>
      nyxBotApi.updateAgent(id, body),
  );
}

export function useSetNyxBotAgentGrants() {
  return useAgentsMutation(({ id, ...grants }: AssistantAgentGrants & { id: string }) =>
    nyxBotApi.setGrants(id, grants),
  );
}

/** Destroying also disconnects the agent's channel bots. */
export function useDestroyNyxBotAgent() {
  return useAgentsMutation((id: string) => nyxBotApi.destroyAgent(id), true);
}

export function useDeleteNyxBotAgent() {
  return useAgentsMutation((id: string) => nyxBotApi.deleteAgent(id), true);
}

export function useForgetNyxBotMemory() {
  return useAgentsMutation(({ agentId, noteId }: { agentId: string; noteId: string }) =>
    nyxBotApi.forget(agentId, noteId),
  );
}

export function useNyxBotSettings(enabled = true) {
  const userId = useAuthStore((state) => state.user?.id);
  return useQuery({
    queryKey: nyxBotQueryKeys.settings(userId),
    queryFn: () => nyxBotApi.settings(),
    enabled: enabled && Boolean(userId),
    retry: false,
  });
}

export function useUpdateNyxBotSettings() {
  const userId = useAuthStore((state) => state.user?.id);
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (update: NyxAgentSettingsUpdate) => nyxBotApi.updateSettings(update),
    onSuccess: (settings) => {
      queryClient.setQueryData(nyxBotQueryKeys.settings(userId), settings);
    },
  });
}

export function useNyxBotChannels(enabled = true) {
  const userId = useAuthStore((state) => state.user?.id);
  return useQuery({
    queryKey: nyxBotQueryKeys.channels(userId),
    queryFn: () => nyxBotApi.channels(),
    enabled: enabled && Boolean(userId),
    retry: false,
  });
}

/** Channel changes move bots between agents, so both lists refresh. */
export function useConnectNyxBotChannel() {
  return useAgentsMutation(
    ({ botId, agentId }: { botId: string; agentId?: string }) =>
      nyxBotApi.connectChannel(botId, agentId),
    true,
  );
}

export function useLinkNyxBotChannel() {
  return useAgentsMutation(
    ({ channelAgentId, agentId }: { channelAgentId: string; agentId: string }) =>
      nyxBotApi.linkChannel(channelAgentId, agentId),
    true,
  );
}

export function useSetNyxBotPrivateChats() {
  return useAgentsMutation(
    ({
      channelAgentId,
      privateChats,
    }: {
      channelAgentId: string;
      privateChats: "owner" | "everyone";
    }) => nyxBotApi.setPrivateChats(channelAgentId, privateChats),
    true,
  );
}

/** The chats a channel bot is in (loaded when its list is opened). */
export function useNyxBotChannelChats(channelAgentId: string, enabled = true) {
  const userId = useAuthStore((state) => state.user?.id);
  return useQuery({
    queryKey: nyxBotQueryKeys.channelChats(userId, channelAgentId),
    queryFn: () => nyxBotApi.channelChats(channelAgentId),
    enabled: enabled && Boolean(userId),
    retry: false,
  });
}

/** A chat's own agent starts a new thread, so thread lists refresh too. */
export function useUpdateNyxBotChannelChat() {
  const userId = useAuthStore((state) => state.user?.id);
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({
      channelAgentId,
      chatId,
      settings,
    }: {
      channelAgentId: string;
      chatId: string;
      settings: NyxAgentChannelChatSettings;
    }) => nyxBotApi.updateChannelChat(channelAgentId, chatId, settings),
    onSettled: async () => {
      await Promise.all([
        queryClient.invalidateQueries({ queryKey: nyxBotQueryKeys.channels(userId) }),
        queryClient.invalidateQueries({ queryKey: nyxBotQueryKeys.threads(userId) }),
      ]);
    },
  });
}

export function useDisconnectNyxBotChannel() {
  return useAgentsMutation(
    (channelAgentId: string) => nyxBotApi.disconnectChannel(channelAgentId),
    true,
  );
}

/**
 * The agent a screen is about: the open thread's agent, else the agent named
 * in the URL, else NyxBot.
 */
export function selectedAgentOf(
  agents: readonly AssistantAgent[] | undefined,
  conversationAgentId: string | undefined,
  searchAgentId: string | undefined,
): AssistantAgent | undefined {
  const id = conversationAgentId ?? searchAgentId;
  return (id ? agents?.find((agent) => agent.id === id) : undefined) ?? nyxBotOf(agents);
}
