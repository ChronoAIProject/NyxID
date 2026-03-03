import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api } from "@/lib/api-client";
import type {
  ServiceAccount,
  ServiceAccountListResponse,
  CreateServiceAccountRequest,
  CreateServiceAccountResponse,
  UpdateServiceAccountRequest,
  RotateSecretResponse,
  RevokeTokensResponse,
  AdminActionResponse,
  SaProviderToken,
  SaProviderListResponse,
  SaProviderActionResponse,
  SaOAuthInitiateResponse,
  SaDeviceCodeInitiateResponse,
  SaDeviceCodePollRequest,
  SaDeviceCodePollResponse,
  SaServiceConnection,
  SaServiceConnectionListResponse,
  SaServiceConnectResponse,
  SaServiceConnectionActionResponse,
} from "@/types/service-accounts";

export function useServiceAccounts(page: number, perPage: number, search?: string) {
  return useQuery({
    queryKey: ["admin", "service-accounts", page, perPage, search],
    queryFn: async (): Promise<ServiceAccountListResponse> => {
      const params = new URLSearchParams({
        page: String(page),
        per_page: String(perPage),
      });
      if (search) params.set("search", search);
      return api.get<ServiceAccountListResponse>(
        `/admin/service-accounts?${params.toString()}`,
      );
    },
  });
}

export function useServiceAccount(saId: string) {
  return useQuery({
    queryKey: ["admin", "service-accounts", saId],
    queryFn: async (): Promise<ServiceAccount> => {
      return api.get<ServiceAccount>(`/admin/service-accounts/${saId}`);
    },
    enabled: saId.length > 0,
  });
}

export function useCreateServiceAccount() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async (
      data: CreateServiceAccountRequest,
    ): Promise<CreateServiceAccountResponse> => {
      return api.post<CreateServiceAccountResponse>(
        "/admin/service-accounts",
        data,
      );
    },
    onSuccess: () => {
      void queryClient.invalidateQueries({
        queryKey: ["admin", "service-accounts"],
      });
    },
  });
}

export function useUpdateServiceAccount() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async ({
      saId,
      data,
    }: {
      readonly saId: string;
      readonly data: UpdateServiceAccountRequest;
    }): Promise<ServiceAccount> => {
      return api.put<ServiceAccount>(
        `/admin/service-accounts/${saId}`,
        data,
      );
    },
    onSuccess: (_, { saId }) => {
      void queryClient.invalidateQueries({
        queryKey: ["admin", "service-accounts"],
      });
      void queryClient.invalidateQueries({
        queryKey: ["admin", "service-accounts", saId],
      });
    },
  });
}

export function useDeleteServiceAccount() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async (saId: string): Promise<AdminActionResponse> => {
      return api.delete<AdminActionResponse>(
        `/admin/service-accounts/${saId}`,
      );
    },
    onSuccess: () => {
      void queryClient.invalidateQueries({
        queryKey: ["admin", "service-accounts"],
      });
    },
  });
}

export function useRotateSecret() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async (saId: string): Promise<RotateSecretResponse> => {
      return api.post<RotateSecretResponse>(
        `/admin/service-accounts/${saId}/rotate-secret`,
      );
    },
    onSuccess: (_data, saId) => {
      void queryClient.invalidateQueries({
        queryKey: ["admin", "service-accounts"],
      });
      void queryClient.invalidateQueries({
        queryKey: ["admin", "service-accounts", saId],
      });
    },
  });
}

export function useRevokeTokens() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async (saId: string): Promise<RevokeTokensResponse> => {
      return api.post<RevokeTokensResponse>(
        `/admin/service-accounts/${saId}/revoke-tokens`,
      );
    },
    onSuccess: (_data, saId) => {
      void queryClient.invalidateQueries({
        queryKey: ["admin", "service-accounts", saId],
      });
    },
  });
}

export function useSaProviders(saId: string) {
  return useQuery({
    queryKey: ["admin", "service-accounts", saId, "providers"],
    queryFn: async (): Promise<readonly SaProviderToken[]> => {
      const res = await api.get<SaProviderListResponse>(
        `/admin/service-accounts/${saId}/providers`,
      );
      return res.tokens;
    },
    enabled: saId.length > 0,
  });
}

export function useConnectApiKeyForSa() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async ({
      saId,
      providerId,
      apiKey,
      label,
    }: {
      readonly saId: string;
      readonly providerId: string;
      readonly apiKey: string;
      readonly label?: string;
    }): Promise<SaProviderActionResponse> => {
      return api.post<SaProviderActionResponse>(
        `/admin/service-accounts/${saId}/providers/${providerId}/connect/api-key`,
        { api_key: apiKey, label },
      );
    },
    onSuccess: (_, { saId }) => {
      void queryClient.invalidateQueries({
        queryKey: ["admin", "service-accounts", saId, "providers"],
      });
    },
  });
}

export function useDisconnectSaProvider() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async ({
      saId,
      providerId,
    }: {
      readonly saId: string;
      readonly providerId: string;
    }): Promise<SaProviderActionResponse> => {
      return api.delete<SaProviderActionResponse>(
        `/admin/service-accounts/${saId}/providers/${providerId}/disconnect`,
      );
    },
    onSuccess: (_, { saId }) => {
      void queryClient.invalidateQueries({
        queryKey: ["admin", "service-accounts", saId, "providers"],
      });
    },
  });
}

export function useInitiateOAuthForSa() {
  return useMutation({
    mutationFn: async ({
      saId,
      providerId,
    }: {
      readonly saId: string;
      readonly providerId: string;
    }): Promise<SaOAuthInitiateResponse> => {
      return api.get<SaOAuthInitiateResponse>(
        `/admin/service-accounts/${saId}/providers/${providerId}/connect/oauth`,
      );
    },
  });
}

export function useInitiateDeviceCodeForSa() {
  return useMutation({
    mutationFn: async ({
      saId,
      providerId,
    }: {
      readonly saId: string;
      readonly providerId: string;
    }): Promise<SaDeviceCodeInitiateResponse> => {
      return api.post<SaDeviceCodeInitiateResponse>(
        `/admin/service-accounts/${saId}/providers/${providerId}/connect/device-code/initiate`,
      );
    },
  });
}

export function usePollDeviceCodeForSa() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async ({
      saId,
      providerId,
      state,
    }: {
      readonly saId: string;
      readonly providerId: string;
      readonly state: string;
    }): Promise<SaDeviceCodePollResponse> => {
      return api.post<SaDeviceCodePollResponse>(
        `/admin/service-accounts/${saId}/providers/${providerId}/connect/device-code/poll`,
        { state } satisfies SaDeviceCodePollRequest,
      );
    },
    onSuccess: (data, { saId }) => {
      if (data.status === "complete") {
        void queryClient.invalidateQueries({
          queryKey: ["admin", "service-accounts", saId, "providers"],
        });
      }
    },
  });
}

export function useSaConnections(saId: string) {
  return useQuery({
    queryKey: ["admin", "service-accounts", saId, "connections"],
    queryFn: async (): Promise<readonly SaServiceConnection[]> => {
      const res = await api.get<SaServiceConnectionListResponse>(
        `/admin/service-accounts/${saId}/connections`,
      );
      return res.connections;
    },
    enabled: saId.length > 0,
  });
}

export function useConnectSaService() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async ({
      saId,
      serviceId,
      credential,
      credentialLabel,
    }: {
      readonly saId: string;
      readonly serviceId: string;
      readonly credential?: string;
      readonly credentialLabel?: string;
    }): Promise<SaServiceConnectResponse> => {
      return api.post<SaServiceConnectResponse>(
        `/admin/service-accounts/${saId}/connections/${serviceId}`,
        { credential, credential_label: credentialLabel },
      );
    },
    onSuccess: (_, { saId }) => {
      void queryClient.invalidateQueries({
        queryKey: ["admin", "service-accounts", saId, "connections"],
      });
    },
  });
}

export function useUpdateSaConnectionCredential() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async ({
      saId,
      serviceId,
      credential,
      credentialLabel,
    }: {
      readonly saId: string;
      readonly serviceId: string;
      readonly credential: string;
      readonly credentialLabel?: string;
    }): Promise<SaServiceConnectionActionResponse> => {
      return api.put<SaServiceConnectionActionResponse>(
        `/admin/service-accounts/${saId}/connections/${serviceId}/credential`,
        { credential, credential_label: credentialLabel },
      );
    },
    onSuccess: (_, { saId }) => {
      void queryClient.invalidateQueries({
        queryKey: ["admin", "service-accounts", saId, "connections"],
      });
    },
  });
}

export function useDisconnectSaService() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async ({
      saId,
      serviceId,
    }: {
      readonly saId: string;
      readonly serviceId: string;
    }): Promise<SaServiceConnectionActionResponse> => {
      return api.delete<SaServiceConnectionActionResponse>(
        `/admin/service-accounts/${saId}/connections/${serviceId}`,
      );
    },
    onSuccess: (_, { saId }) => {
      void queryClient.invalidateQueries({
        queryKey: ["admin", "service-accounts", saId, "connections"],
      });
    },
  });
}
