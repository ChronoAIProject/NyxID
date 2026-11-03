import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api } from "@/lib/api-client";
import { internalOwnersSchema } from "@/schemas/assistant-real-work";
import { useAuthStore } from "@/stores/auth-store";
const path = "/admin/assistant/analytics/internal-owners";
export function useAssistantInternalOwners(enabled: boolean) {
  const userId = useAuthStore((s) => s.user?.id);
  const client = useQueryClient();
  const queryKey = ["admin", "assistant-internal-owners", userId];
  const query = useQuery({
    queryKey,
    enabled,
    retry: false,
    queryFn: async () => internalOwnersSchema.parse(await api.get(path)),
  });
  const save = useMutation({
    mutationFn: async (owner_ids: string[]) =>
      internalOwnersSchema.parse(await api.put(path, { owner_ids })),
    onSuccess: async (data) => {
      client.setQueryData(queryKey, data);
      await client.invalidateQueries({
        queryKey: ["admin", "assistant-analytics"],
      });
    },
  });
  return { query, save };
}
