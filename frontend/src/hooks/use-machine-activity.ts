import { useQuery, useQueryClient } from "@tanstack/react-query";
import { api } from "@/lib/api-client";
import { assistantJson } from "@/lib/assistant/assistant-http";
import { machineActivityPageSchema } from "@/schemas/machine-activity";

export function useMachineActivity(
  nodeId: string,
  agent: string,
  before?: string,
) {
  return useQuery({
    queryKey: ["machine-activity", nodeId, agent, before],
    queryFn: async () => {
      const params = new URLSearchParams({ limit: "50" });
      if (agent === "unknown") params.set("unattributed", "true");
      else if (agent) params.set("agent_id", agent);
      if (before) params.set("before", before);
      return machineActivityPageSchema.parse(
        await api.get(
          `/machines/${encodeURIComponent(nodeId)}/activity?${params}`,
        ),
      );
    },
    refetchInterval: before ? false : 5000,
  });
}

export function useMachinePreviewPolicy(
  conversationId: string | undefined,
  open: boolean,
) {
  const client = useQueryClient();
  const key = ["machine-preview-policy", conversationId];
  const endpoint = `/assistant/nyxagent/conversations/${encodeURIComponent(conversationId ?? "")}/machine-preview-policy`;
  const query = useQuery({
    queryKey: key,
    queryFn: () => assistantJson<{ enabled: boolean }>(endpoint),
    enabled: !!conversationId && open,
  });
  async function change(enabled: boolean) {
    if (!conversationId) return;
    const updated = await assistantJson<{ enabled: boolean }>(endpoint, {
      method: "PUT",
      body: { enabled },
    });
    client.setQueryData(key, updated);
  }
  return { query, change };
}
