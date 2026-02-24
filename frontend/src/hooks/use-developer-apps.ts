import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api } from "@/lib/api-client";
import type { OAuthClient } from "@/types/api";

interface DeveloperOAuthClientListResponse {
  readonly clients: readonly OAuthClient[];
}

export interface CreateDeveloperAppRequest {
  readonly name: string;
  readonly redirect_uris: readonly string[];
  readonly client_type: "public" | "confidential";
  readonly delegation_scopes?: string;
}

export function useDeveloperApps() {
  return useQuery({
    queryKey: ["developer", "oauth-clients"],
    queryFn: async (): Promise<DeveloperOAuthClientListResponse> => {
      return api.get<DeveloperOAuthClientListResponse>("/developer/oauth-clients");
    },
  });
}

export function useDeveloperApp(clientId: string) {
  return useQuery({
    queryKey: ["developer", "oauth-clients", clientId],
    queryFn: async (): Promise<OAuthClient> => {
      return api.get<OAuthClient>(`/developer/oauth-clients/${clientId}`);
    },
    enabled: clientId.length > 0,
  });
}

export function useCreateDeveloperApp() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async (data: CreateDeveloperAppRequest): Promise<OAuthClient> => {
      return api.post<OAuthClient>("/developer/oauth-clients", data);
    },
    onSuccess: (created) => {
      void queryClient.invalidateQueries({
        queryKey: ["developer", "oauth-clients"],
      });
      void queryClient.setQueryData(
        ["developer", "oauth-clients", created.id],
        created,
      );
    },
  });
}

export interface UpdateDeveloperAppRequest {
  readonly name?: string;
  readonly redirect_uris?: readonly string[];
  readonly delegation_scopes?: string;
}

interface RotateSecretResponse {
  readonly id: string;
  readonly client_secret: string;
}

export function useUpdateDeveloperApp() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async ({
      clientId,
      data,
    }: {
      readonly clientId: string;
      readonly data: UpdateDeveloperAppRequest;
    }): Promise<OAuthClient> => {
      return api.patch<OAuthClient>(`/developer/oauth-clients/${clientId}`, data);
    },
    onSuccess: (updated) => {
      void queryClient.invalidateQueries({
        queryKey: ["developer", "oauth-clients"],
      });
      void queryClient.setQueryData(
        ["developer", "oauth-clients", updated.id],
        updated,
      );
    },
  });
}

export function useDeleteDeveloperApp() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async (clientId: string): Promise<{ message: string }> => {
      return api.delete<{ message: string }>(`/developer/oauth-clients/${clientId}`);
    },
    onSuccess: () => {
      void queryClient.invalidateQueries({
        queryKey: ["developer", "oauth-clients"],
      });
    },
  });
}

export function useRotateDeveloperAppSecret() {
  return useMutation({
    mutationFn: async (clientId: string): Promise<RotateSecretResponse> => {
      return api.post<RotateSecretResponse>(
        `/developer/oauth-clients/${clientId}/rotate-secret`,
      );
    },
  });
}
