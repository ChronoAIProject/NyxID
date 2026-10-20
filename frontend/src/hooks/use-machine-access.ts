import { useRef } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { assistantJson } from "@/lib/assistant/assistant-http";
import {
  machineAccessSchema,
  machineContextSchema,
  type MachineContextSelection,
  type MachineSelection,
} from "@/schemas/machine-access";
import { useAuthStore } from "@/stores/auth-store";
import { nyxBotQueryKeys } from "./use-nyxbot-agents";
import { useAgentOwnerMutation } from "./use-agent-owner-mutation";
const path = (id: string) => `/assistant/nyxagent/agents/${encodeURIComponent(id)}/machines`;
export function useMachineAccess(id: string, enabled = true) {
  const userId = useAuthStore(s => s.user?.id);
  const recovery = useRef({ key: "", until: 0 });
  return useQuery({
    queryKey: [...nyxBotQueryKeys.agents(userId), id, "machine-access"],
    queryFn: async () => machineAccessSchema.array().parse(await assistantJson(path(id))),
    enabled: enabled && Boolean(userId), retry: false,
    refetchInterval: query => {
      if (!enabled || query.state.error) return false;
      const pending = query.state.data?.filter(machine => machine.revocation_pending || machine.delivery_pending) ?? [];
      const key = pending.length ? `${userId}:${id}:${pending.map(machine => `${machine.node_id}:${machine.revision}`).join(",")}` : "";
      if (recovery.current.key !== key) recovery.current = { key, until: Date.now() + 60_000 };
      return key && Date.now() < recovery.current.until ? 3_000 : false;
    },
    refetchIntervalInBackground: false,
  });
}
export function useSetMachineAccess(id: string) {
  const client = useQueryClient();
  return useAgentOwnerMutation({
    mutationFn: ({ node, selection }: { node: string; selection: MachineSelection }, guard) =>
      assistantJson(`${path(id)}/${encodeURIComponent(node)}`, { method: "PUT", body: selection, authorityGuard: guard }),
    onSettled: (owner) => client.invalidateQueries({ queryKey: nyxBotQueryKeys.agents(owner) }),
  });
}

export function useRequestMachineContext(agentId: string) {
  const client = useQueryClient();
  return useAgentOwnerMutation({
    mutationFn: ({ node, selection }: { node: string; selection: MachineContextSelection }, guard) =>
      assistantJson(`${path(agentId)}/${encodeURIComponent(node)}/context-request`, {
        method: "POST",
        body: { selection },
        authorityGuard: guard,
      }),
    onSettled: owner => client.invalidateQueries({ queryKey: nyxBotQueryKeys.agents(owner) }),
  });
}

export function useMachineContexts(nodeId: string | undefined, enabled = true) {
  const userId = useAuthStore(s => s.user?.id);
  return useQuery({
    queryKey: ["machine-contexts", userId, nodeId],
    queryFn: async () =>
      machineContextSchema.array().parse(
        await assistantJson(`/assistant/nyxagent/machines/${encodeURIComponent(nodeId ?? "")}/contexts`),
      ),
    enabled: Boolean(userId) && Boolean(nodeId) && enabled,
    retry: false,
    staleTime: 10_000,
  });
}
