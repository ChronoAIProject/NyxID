import { useLayoutEffect, useMemo } from "react";
import { useQuery } from "@tanstack/react-query";
import { create } from "zustand";
import {
  agentRosterMood,
  type AgentRosterMoodInput,
  type AgentThreadActivity,
} from "../../../shared/agent-avatars/activity";
import {
  processState,
  type AvatarMood,
} from "@/lib/assistant/agent-avatar-model";
import { nyxBotApi } from "@/lib/assistant/nyxbot-api";
import { ApiError } from "@/lib/api-error";
import { subscribeAssistantIdentity } from "@/lib/assistant/identity";
import type { AssistantAgent } from "@/schemas/assistant-nyxagent";
import { useAuthStore } from "@/stores/auth-store";
import { useAgentAvatarsEnabled } from "./use-agent-avatars";
import { nyxBotQueryKeys } from "./use-nyxbot-agents";

type ActivityOverride = NonNullable<AgentRosterMoodInput["override"]>;
type OpenActivity = ActivityOverride & { readonly userId: string };
const useOpenActivity = create<{ readonly observation?: OpenActivity }>(
  () => ({}),
);
subscribeAssistantIdentity(() =>
  useOpenActivity.setState({ observation: undefined }),
);

/** Publish only an identified open thread. Cleanup owns this exact observation. */
export function usePublishAgentActivity(
  userId: string | undefined,
  conversationId: string | undefined,
  agentId: string | undefined,
  mood: AvatarMood,
) {
  const enabled = useAgentAvatarsEnabled();
  useLayoutEffect(() => {
    if (!enabled || !userId || !conversationId || !agentId) return;
    if (useAuthStore.getState().user?.id !== userId) return;
    const observation = {
      userId,
      conversationId,
      agentId,
      state: processState(mood),
    };
    useOpenActivity.setState({ observation });
    return () => {
      if (useOpenActivity.getState().observation === observation) {
        useOpenActivity.setState({ observation: undefined });
      }
    };
  }, [agentId, conversationId, enabled, mood, userId]);
}

export function useAgentActivity(enabled = true) {
  const userId = useAuthStore((state) => state.user?.id);
  const feature = useAgentAvatarsEnabled();
  return useQuery({
    queryKey: nyxBotQueryKeys.activity(userId),
    queryFn: async ({ signal }) => {
      try {
        return await nyxBotApi.activity(signal);
      } catch (error) {
        if (error instanceof ApiError && error.status === 404) return null;
        throw error;
      }
    },
    enabled: (query) =>
      enabled && feature && Boolean(userId) && query.state.data !== null,
    staleTime: Infinity,
    retry: false,
  });
}

interface ActivitySnapshot {
  readonly updatedAt: number;
  readonly agents: ReadonlyMap<string, readonly AgentThreadActivity[]>;
}

export function rosterMoodFor(
  agent: AssistantAgent,
  rosterUpdatedAt = 0,
  snapshot?: ActivitySnapshot,
  override?: ActivityOverride,
) {
  return agentRosterMood({
    agentId: agent.id,
    activity:
      snapshot && snapshot.updatedAt >= rosterUpdatedAt
        ? (snapshot.agents.get(agent.id) ?? [])
        : agent.activity,
    pendingOwnerActions: agent.pending_acknowledgements,
    running: agent.status === "running",
    destroyed: agent.status === "destroyed",
    override,
  });
}

export type AgentActivityMood = (
  agent: AssistantAgent,
) => ReturnType<typeof rosterMoodFor>;

/** A new endpoint snapshot includes the absence of work for unlisted agents. */
export function useRosterActivity(
  rosterUpdatedAt: number,
  enabled = true,
): AgentActivityMood {
  const userId = useAuthStore((state) => state.user?.id);
  const feature = useAgentAvatarsEnabled();
  const override = useOpenActivity((state) =>
    state.observation?.userId === userId ? state.observation : undefined,
  );
  const { data, dataUpdatedAt } = useAgentActivity(enabled);
  return useMemo(() => {
    const snapshot =
      data && feature
        ? {
            updatedAt: dataUpdatedAt,
            agents: new Map(
              data.agents.map((agent) => [agent.agent_id, agent.threads]),
            ),
          }
        : undefined;
    return (agent) =>
      rosterMoodFor(
        agent,
        rosterUpdatedAt,
        snapshot,
        feature ? override : undefined,
      );
  }, [data, dataUpdatedAt, feature, override, rosterUpdatedAt]);
}
