import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { assistantJson } from "@/lib/assistant/assistant-http";
import { machineAccessSchema, type MachineSelection } from "@/schemas/machine-access";
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
