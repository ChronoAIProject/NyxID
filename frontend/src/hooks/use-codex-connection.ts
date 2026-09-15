import { useIsMutating, useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api, apiClient } from "@/lib/api-client";
import { codexConnectionSchema, type CodexConnection } from "@/schemas/codex-connection";

const endpoint = "/providers/codex-connection";
const queryKey = ["codex-connection"];
const mutationKey = ["verify-codex-connection"];

function sameConnection(left: CodexConnection | undefined, right: CodexConnection) {
  return left?.account_id === right.account_id
    && left.connection?.id === right.connection?.id
    && left.connection?.state_version === right.connection?.state_version
    && left.service_id === right.service_id;
}

export function useCodexConnection(enabled: boolean) {
  const verifying = useIsMutating({ mutationKey }) > 0;
  return useQuery({ queryKey, enabled: enabled && !verifying, staleTime: 30_000,
    queryFn: async ({ signal }) => codexConnectionSchema.parse(await apiClient(endpoint, { signal })),
  });
}

export function useVerifyCodexConnection() {
  const client = useQueryClient();
  return useMutation({ mutationKey,
    mutationFn: async (reviewed: CodexConnection) =>
      codexConnectionSchema.parse(await api.post(`${endpoint}/verify`, { connection: reviewed.connection, model: "gpt-4.1-mini" })),
    onMutate: async (reviewed) => {
      await client.cancelQueries({ queryKey });
      client.setQueryData<CodexConnection>(queryKey,
        (saved) => saved && sameConnection(saved, reviewed) ? { ...saved, status: "saved" } : saved);
    },
    onSuccess: async (data, reviewed) => {
      await client.cancelQueries({ queryKey });
      client.setQueryData<CodexConnection>(queryKey,
        (saved) => sameConnection(saved, reviewed) && sameConnection(data, reviewed) ? data : saved);
    },
  });
}
