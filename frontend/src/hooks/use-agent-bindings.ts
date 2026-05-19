import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api } from "@/lib/api-client";
import type {
  AgentServiceBinding,
  AgentServiceBindingListResponse,
} from "@/types/keys";

export function useAgentBindings(keyId: string) {
  return useQuery({
    queryKey: ["agent-bindings", keyId],
    queryFn: async (): Promise<readonly AgentServiceBinding[]> => {
      const res = await api.get<AgentServiceBindingListResponse>(
        `/api-keys/${keyId}/bindings`,
      );
      return res.bindings;
    },
    enabled: Boolean(keyId),
  });
}

interface CreateBindingParams {
  readonly keyId: string;
  readonly user_service_id: string;
  readonly user_api_key_id: string;
}

export function useCreateBinding() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async (
      params: CreateBindingParams,
    ): Promise<AgentServiceBinding> => {
      const { keyId, ...body } = params;
      return api.post<AgentServiceBinding>(
        `/api-keys/${keyId}/bindings`,
        body,
      );
    },
    onSuccess: (_data, variables) => {
      void queryClient.invalidateQueries({
        queryKey: ["agent-bindings", variables.keyId],
      });
      void queryClient.invalidateQueries({ queryKey: ["api-keys"] });
      void queryClient.invalidateQueries({
        queryKey: ["api-keys", variables.keyId],
      });
    },
  });
}

export function useDeleteBinding() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async ({
      keyId,
      bindingId,
    }: {
      readonly keyId: string;
      readonly bindingId: string;
    }): Promise<void> => {
      return api.delete<void>(`/api-keys/${keyId}/bindings/${bindingId}`);
    },
    onSuccess: (_data, variables) => {
      void queryClient.invalidateQueries({
        queryKey: ["agent-bindings", variables.keyId],
      });
      void queryClient.invalidateQueries({ queryKey: ["api-keys"] });
      void queryClient.invalidateQueries({
        queryKey: ["api-keys", variables.keyId],
      });
    },
  });
}
