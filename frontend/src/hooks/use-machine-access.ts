import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { assistantJson } from "@/lib/assistant/assistant-http";
import {
  machineAccessSchema,
  machineContextSchema,
  type MachineContextSelection,
  type MachineSelection,
} from "@/schemas/machine-access";
import { useAuthStore } from "@/stores/auth-store";
import { nyxBotQueryKeys } from "./use-nyxbot-agents";
const path = (id: string) => `/assistant/nyxagent/agents/${encodeURIComponent(id)}/machines`;
export function useMachineAccess(id: string, enabled = true) {
  const userId = useAuthStore(s => s.user?.id);
  return useQuery({
    queryKey: [...nyxBotQueryKeys.agents(userId), id, "machine-access"],
    queryFn: async () => machineAccessSchema.array().parse(await assistantJson(path(id))),
    enabled: enabled && Boolean(userId), retry: false,
  });
}
export function useSetMachineAccess(id: string) {
  const client = useQueryClient();
  const userId = useAuthStore(s => s.user?.id);
  return useMutation({
    mutationFn: ({ node, selection }: { node: string; selection: MachineSelection }) =>
      assistantJson(`${path(id)}/${encodeURIComponent(node)}`, { method: "PUT", body: selection }),
    onSuccess: () => client.invalidateQueries({ queryKey: nyxBotQueryKeys.agents(userId) }),
  });
}

export function useRequestMachineContext(agentId: string) {
  const client = useQueryClient();
  const userId = useAuthStore(s => s.user?.id);
  return useMutation({
    mutationFn: ({ node, selection }: { node: string; selection: MachineContextSelection }) =>
      assistantJson(`${path(agentId)}/${encodeURIComponent(node)}/context-request`, {
        method: "POST",
        body: { selection },
      }),
    onSuccess: () => client.invalidateQueries({ queryKey: nyxBotQueryKeys.agents(userId) }),
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
