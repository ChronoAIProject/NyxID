import { useRef } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { assistantJson } from "@/lib/assistant/assistant-http";
import {
  machineAccessSchema,
  machineContextSchema,
  machineContextPrepareSchema,
  machineContextReviewSchema,
  type MachineContextPrepare,
  type MachineSelection,
} from "@/schemas/machine-access";
import { useAuthStore } from "@/stores/auth-store";
import { nyxBotQueryKeys } from "./use-nyxbot-agents";
import { useAgentOwnerMutation } from "./use-agent-owner-mutation";
import { hasPendingMachineApplication } from "@/lib/machine-access";
const path = (id: string) =>
  `/assistant/nyxagent/agents/${encodeURIComponent(id)}/machines`;
export function useMachineAccess(id: string, enabled = true) {
  const userId = useAuthStore((s) => s.user?.id);
  const recovery = useRef({ key: "", until: 0 });
  return useQuery({
    queryKey: [...nyxBotQueryKeys.agents(userId), id, "machine-access"],
    queryFn: async () =>
      machineAccessSchema.array().parse(await assistantJson(path(id))),
    enabled: enabled && Boolean(userId),
    retry: false,
    refetchInterval: (query) => {
      if (!enabled || query.state.error) return false;
      const pending =
        query.state.data?.filter(hasPendingMachineApplication) ?? [];
      const key = pending.length
        ? `${userId}:${id}:${pending.map((machine) => `${machine.node_id}:${machine.revision}:${machine.machine_configuration?.revision}:${machine.machine_configuration?.runtime_generation}`).join(",")}`
        : "";
      if (recovery.current.key !== key)
        recovery.current = { key, until: Date.now() + 60_000 };
      return key && Date.now() < recovery.current.until ? 3_000 : false;
    },
    refetchIntervalInBackground: false,
  });
}
export function useSetMachineAccess(id: string) {
  const client = useQueryClient();
  return useAgentOwnerMutation({
    mutationFn: (
      { node, selection }: { node: string; selection: MachineSelection },
      guard,
    ) =>
      assistantJson(`${path(id)}/${encodeURIComponent(node)}`, {
        method: "PUT",
        body: selection,
        authorityGuard: guard,
      }),
    onSettled: (owner) =>
      client.invalidateQueries({ queryKey: nyxBotQueryKeys.agents(owner) }),
  });
}

const reviewPath = (agent: string, node: string) =>
  `${path(agent)}/${encodeURIComponent(node)}`;
const reviewKey = (
  owner: string | undefined,
  agent: string,
  node: string,
  review: string | undefined,
) => [
  ...nyxBotQueryKeys.agents(owner),
  agent,
  "machine-context-review",
  node,
  review,
];
function parseReview(
  value: unknown,
  agent: string,
  node: string,
  review?: string,
) {
  const result = machineContextReviewSchema.parse(value);
  if (
    result.agent_id !== agent ||
    result.node_id !== node ||
    (review && result.review_id !== review)
  ) {
    throw new Error(
      "The machine context review target does not match this request. Prepare and review again.",
    );
  }
  return result;
}

export function usePrepareMachineContext(agentId: string) {
  const client = useQueryClient();
  return useAgentOwnerMutation({
    mutationFn: async (
      { node, body }: { node: string; body: MachineContextPrepare },
      guard,
    ) => {
      const reviewed = machineContextPrepareSchema.parse(body);
      const result = parseReview(
        await assistantJson(`${reviewPath(agentId, node)}/context-prepare`, {
          method: "POST",
          body: reviewed,
          authorityGuard: guard,
        }),
        agentId,
        node,
      );
      if (result.mode !== reviewed.mode)
        throw new Error(
          "The machine context review target mode changed. Prepare and review again.",
        );
      return result;
    },
    onSettled: (owner) =>
      client.invalidateQueries({ queryKey: nyxBotQueryKeys.agents(owner) }),
  });
}

export function useMachineContextReview(
  agentId: string,
  node: string,
  reviewId: string | undefined,
) {
  const owner = useAuthStore((s) => s.user?.id);
  const recovery = useRef({ key: "", until: 0 });
  return useQuery({
    queryKey: reviewKey(owner, agentId, node, reviewId),
    queryFn: async () =>
      parseReview(
        await assistantJson(
          `${reviewPath(agentId, node)}/context-reviews/${encodeURIComponent(reviewId ?? "")}`,
        ),
        agentId,
        node,
        reviewId,
      ),
    enabled: Boolean(owner && reviewId),
    retry: false,
    refetchInterval: (query) => {
      const row = query.state.data;
      if (
        !row ||
        query.state.error ||
        row.status === "failed" ||
        (row.status !== "committed" &&
          (!row.applicable || Date.parse(row.expires_at) <= Date.now()))
      )
        return false;
      const pending =
        row.status !== "committed" ||
        !row.authorization_applied ||
        (row.machine_configuration !== null &&
          row.machine_configuration.state !== "applied");
      const key = pending
        ? `${owner}:${agentId}:${node}:${reviewId}:${row.status}`
        : "";
      if (key !== recovery.current.key)
        recovery.current = { key, until: Date.now() + 60_000 };
      return key && Date.now() < recovery.current.until ? 3_000 : false;
    },
    refetchOnWindowFocus: true,
    refetchIntervalInBackground: false,
  });
}

export function useCommitMachineContext(agentId: string) {
  const client = useQueryClient();
  return useAgentOwnerMutation({
    mutationFn: async (
      { node, reviewId }: { node: string; reviewId: string },
      guard,
    ) => {
      const result = parseReview(
        await assistantJson(
          `${reviewPath(agentId, node)}/context-reviews/${encodeURIComponent(reviewId)}/commit`,
          {
            method: "POST",
            body: {},
            authorityGuard: guard,
          },
        ),
        agentId,
        node,
        reviewId,
      );
      guard();
      client.setQueryData(
        reviewKey(useAuthStore.getState().user?.id, agentId, node, reviewId),
        result,
      );
      return result;
    },
    onSettled: async (owner) => {
      await Promise.all([
        client.invalidateQueries({ queryKey: nyxBotQueryKeys.agents(owner) }),
        client.invalidateQueries({ queryKey: ["remote-machines", owner] }),
      ]);
    },
  });
}

export function useMachineContexts(nodeId: string | undefined, enabled = true) {
  const userId = useAuthStore((s) => s.user?.id);
  return useQuery({
    queryKey: ["machine-contexts", userId, nodeId],
    queryFn: async () =>
      machineContextSchema
        .array()
        .parse(
          await assistantJson(
            `/assistant/nyxagent/machines/${encodeURIComponent(nodeId ?? "")}/contexts`,
          ),
        ),
    enabled: Boolean(userId) && Boolean(nodeId) && enabled,
    retry: false,
    staleTime: 10_000,
  });
}
