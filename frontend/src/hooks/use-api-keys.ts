import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api } from "@/lib/api-client";
import type { ApiKey, ApiKeyCreateResponse } from "@/types/api";
import type { CreateApiKeyFormData } from "@/schemas/api-keys";

export function useApiKeys() {
  return useQuery({
    queryKey: ["api-keys"],
    queryFn: async (): Promise<readonly ApiKey[]> => {
      return api.get<readonly ApiKey[]>("/api-keys");
    },
  });
}

export function useCreateApiKey() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async (
      data: CreateApiKeyFormData,
    ): Promise<ApiKeyCreateResponse> => {
      return api.post<ApiKeyCreateResponse>("/api-keys", data);
    },
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ["api-keys"] });
    },
  });
}

export function useDeleteApiKey() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async (id: string): Promise<void> => {
      return api.delete<void>(`/api-keys/${id}`);
    },
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ["api-keys"] });
    },
  });
}

export function useRotateApiKey() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async (id: string): Promise<ApiKeyCreateResponse> => {
      return api.post<ApiKeyCreateResponse>(`/api-keys/${id}/rotate`);
    },
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ["api-keys"] });
    },
  });
}
