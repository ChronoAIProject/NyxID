import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { assistantJson } from "@/lib/assistant/assistant-http";
import {
  agentServiceOperationsSchema,
  type OperationSelection,
} from "@/schemas/agent-operation-scopes";
import { useAuthStore } from "@/stores/auth-store";
import { nyxBotQueryKeys } from "./use-nyxbot-agents";

const path = (id: string, guests = false) =>
  `/assistant/nyxagent/agents/${encodeURIComponent(id)}/${guests ? "guest-operations" : "operations"}`;
export function useAgentOperations(id: string, enabled = true, guests = false) {
  const userId = useAuthStore((state) => state.user?.id);
  return useQuery({
    queryKey: [...nyxBotQueryKeys.agents(userId), id, guests ? "guest-operations" : "operations"],
    queryFn: async () =>
      agentServiceOperationsSchema.array().parse(await assistantJson(path(id, guests))),
    enabled: enabled && Boolean(userId),
    retry: false,
  });
}
export function useSetAgentOperations(id: string, guests = false) {
  const client = useQueryClient();
  const userId = useAuthStore((state) => state.user?.id);
  return useMutation({
    mutationFn: ({
      serviceId,
      selection,
    }: {
      serviceId: string;
      selection: OperationSelection;
    }) =>
      assistantJson(`${path(id, guests)}/${encodeURIComponent(serviceId)}`, {
        method: "PUT",
        body: selection,
      }),
    onSuccess: () =>
      client.invalidateQueries({ queryKey: nyxBotQueryKeys.agents(userId) }),
  });
}
