import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api } from "@/lib/api-client";
import {
  agentServiceOperationsSchema,
  type OperationSelection,
} from "@/schemas/agent-operation-scopes";

const operationsKey = (keyId: string) =>
  ["api-keys", keyId, "operations"] as const;

export function useApiKeyOperations(keyId: string, enabled = true) {
  return useQuery({
    queryKey: operationsKey(keyId),
    queryFn: async () =>
      agentServiceOperationsSchema
        .array()
        .parse(
          await api.get<unknown>(
            `/api-keys/${encodeURIComponent(keyId)}/operations`,
          ),
        ),
    enabled: enabled && Boolean(keyId),
    retry: false,
  });
}

export function useSetApiKeyOperations(keyId: string) {
  const client = useQueryClient();
  return useMutation({
    mutationFn: ({
      serviceId,
      selection,
    }: {
      readonly serviceId: string;
      readonly selection: OperationSelection;
    }) =>
      api.put<unknown>(
        `/api-keys/${encodeURIComponent(keyId)}/operations/${encodeURIComponent(serviceId)}`,
        selection,
      ),
    onSuccess: () =>
      client.invalidateQueries({ queryKey: operationsKey(keyId) }),
  });
}
