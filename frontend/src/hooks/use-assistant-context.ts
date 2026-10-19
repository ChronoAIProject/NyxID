import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { assistantJson } from "@/lib/assistant/assistant-http";
import { assistantContextSchema } from "@/schemas/assistant-context";

const path = (conversationId: string) =>
  `/assistant/nyxagent/conversations/${conversationId}/context`;
const key = (conversationId: string) => ["assistant", "context", conversationId];

/** The working context of one conversation. Polled while a turn runs so the
 * pending actions and the checkpoint stay current. */
export function useAssistantContext(conversationId: string, enabled: boolean) {
  return useQuery({
    queryKey: key(conversationId),
    queryFn: async () => assistantContextSchema.parse(await assistantJson(path(conversationId))),
    enabled,
    refetchInterval: (query) => (query.state.data?.active_turn_id ? 3_000 : false),
  });
}

export function useResetAssistantContext(conversationId: string, onReset?: () => void) {
  const client = useQueryClient();
  return useMutation({
    mutationFn: async () =>
      assistantContextSchema.parse(
        await assistantJson(`${path(conversationId)}/reset`, { method: "POST" }),
      ),
    onSuccess: (context) => {
      client.setQueryData(key(conversationId), context);
      onReset?.();
    },
  });
}
