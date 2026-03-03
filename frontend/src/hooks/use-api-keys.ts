import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api } from "@/lib/api-client";
import type { ApiKey, ApiKeyCreateResponse } from "@/types/api";
import type { CreateApiKeyFormData } from "@/schemas/api-keys";

export function useApiKeys() {
  return useQuery({
    queryKey: ["api-keys"],
    queryFn: async (): Promise<readonly ApiKey[]> => {
      const res = await api.get<{ readonly keys: readonly ApiKey[] }>(
        "/api-keys",
      );
      return res.keys;
    },
  });
}

export function useCreateApiKey() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async (
      data: CreateApiKeyFormData,
    ): Promise<ApiKeyCreateResponse> => {
      // Backend expects scopes as a space-separated string, not an array
      const payload = {
        name: data.name,
        scopes: data.scopes.join(" "),
        expires_at: data.expires_at ?? null,
      };
      return api.post<ApiKeyCreateResponse>("/api-keys", payload);
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
