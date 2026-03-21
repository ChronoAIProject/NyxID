import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api } from "@/lib/api-client";
import type {
  ProviderConfig,
  ProviderListResponse,
  UserProviderToken,
  UserTokenListResponse,
  UserProviderCredentials,
  OAuthInitiateResponse,
  DeviceCodeInitiateResponse,
  DeviceCodePollRequest,
  DeviceCodePollResponse,
  ServiceProviderRequirement,
  TelegramWidgetConfig,
  TelegramLoginData,
} from "@/types/api";

export function useProviders() {
  return useQuery({
    queryKey: ["providers"],
    queryFn: async (): Promise<readonly ProviderConfig[]> => {
      const res = await api.get<ProviderListResponse>("/providers");
      return res.providers;
    },
  });
}

export function useProvider(providerId: string) {
  return useQuery({
    queryKey: ["providers", providerId],
    queryFn: async (): Promise<ProviderConfig> => {
      return api.get<ProviderConfig>(`/providers/${providerId}`);
    },
    enabled: providerId.length > 0,
  });
}

export function useMyProviderTokens() {
  return useQuery({
    queryKey: ["provider-tokens"],
    queryFn: async (): Promise<readonly UserProviderToken[]> => {
      const res = await api.get<UserTokenListResponse>("/providers/my-tokens");
      return res.tokens;
    },
  });
}

export function useConnectApiKey() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async ({
      providerId,
      apiKey,
      label,
    }: {
      readonly providerId: string;
      readonly apiKey: string;
      readonly label?: string;
    }): Promise<UserProviderToken> => {
      return api.post<UserProviderToken>(
        `/providers/${providerId}/connect/api-key`,
        { api_key: apiKey, label },
      );
    },
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ["provider-tokens"] });
      void queryClient.invalidateQueries({ queryKey: ["providers"] });
      void queryClient.invalidateQueries({ queryKey: ["llm-status"] });
    },
  });
}

export function useInitiateOAuth() {
  return useMutation({
    mutationFn: async (providerId: string): Promise<OAuthInitiateResponse> => {
      return api.get<OAuthInitiateResponse>(
        `/providers/${providerId}/connect/oauth`,
      );
    },
  });
}

export function useInitiateDeviceCode() {
  return useMutation({
    mutationFn: async (
      providerId: string,
    ): Promise<DeviceCodeInitiateResponse> => {
      return api.post<DeviceCodeInitiateResponse>(
        `/providers/${providerId}/connect/device-code/initiate`,
      );
    },
  });
}

export function usePollDeviceCode() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async ({
      providerId,
      state,
    }: {
      readonly providerId: string;
      readonly state: string;
    }): Promise<DeviceCodePollResponse> => {
      return api.post<DeviceCodePollResponse>(
        `/providers/${providerId}/connect/device-code/poll`,
        { state } satisfies DeviceCodePollRequest,
      );
    },
    onSuccess: (data) => {
      if (data.status === "complete") {
        void queryClient.invalidateQueries({ queryKey: ["provider-tokens"] });
        void queryClient.invalidateQueries({ queryKey: ["providers"] });
        void queryClient.invalidateQueries({ queryKey: ["llm-status"] });
      }
    },
  });
}

export function useDisconnectProvider() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async (providerId: string): Promise<void> => {
      return api.delete<void>(`/providers/${providerId}/disconnect`);
    },
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ["provider-tokens"] });
      void queryClient.invalidateQueries({ queryKey: ["providers"] });
      void queryClient.invalidateQueries({ queryKey: ["llm-status"] });
    },
  });
}

export function useRefreshProviderToken() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async (providerId: string): Promise<void> => {
      return api.post<void>(`/providers/${providerId}/refresh`);
    },
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ["provider-tokens"] });
      void queryClient.invalidateQueries({ queryKey: ["llm-status"] });
    },
  });
}

// --- Telegram Login Widget hooks ---

export function useTelegramWidgetConfig(providerId: string) {
  return useQuery({
    queryKey: ["telegram-widget-config", providerId],
    queryFn: async (): Promise<TelegramWidgetConfig> => {
      return api.get<TelegramWidgetConfig>(
        `/providers/${providerId}/connect/telegram`,
      );
    },
    enabled: providerId.length > 0,
  });
}

export function useConnectTelegramWidget() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async ({
      providerId,
      data,
    }: {
      readonly providerId: string;
      readonly data: TelegramLoginData;
    }): Promise<UserProviderToken> => {
      return api.post<UserProviderToken>(
        `/providers/${providerId}/connect/telegram/callback`,
        data,
      );
    },
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ["provider-tokens"] });
      void queryClient.invalidateQueries({ queryKey: ["providers"] });
      void queryClient.invalidateQueries({ queryKey: ["llm-status"] });
    },
  });
}

// --- User provider credentials hooks ---

export function useMyProviderCredentials(providerId: string) {
  return useQuery({
    queryKey: ["provider-credentials", providerId],
    queryFn: async (): Promise<UserProviderCredentials> => {
      return api.get<UserProviderCredentials>(
        `/providers/${providerId}/credentials`,
      );
    },
    enabled: providerId.length > 0,
  });
}

export function useSetProviderCredentials() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async ({
      providerId,
      client_id,
      client_secret,
      label,
    }: {
      readonly providerId: string;
      readonly client_id: string;
      readonly client_secret?: string;
      readonly label?: string;
    }): Promise<UserProviderCredentials> => {
      return api.put<UserProviderCredentials>(
        `/providers/${providerId}/credentials`,
        { client_id, client_secret, label },
      );
    },
    onSuccess: (_data, variables) => {
      void queryClient.invalidateQueries({
        queryKey: ["provider-credentials", variables.providerId],
      });
      void queryClient.invalidateQueries({ queryKey: ["providers"] });
    },
  });
}

export function useDeleteProviderCredentials() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async (providerId: string): Promise<void> => {
      return api.delete<void>(`/providers/${providerId}/credentials`);
    },
    onSuccess: (_data, providerId) => {
      void queryClient.invalidateQueries({
        queryKey: ["provider-credentials", providerId],
      });
      void queryClient.invalidateQueries({ queryKey: ["providers"] });
    },
  });
}

// --- Admin CRUD hooks ---

export function useCreateProvider() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async (data: {
      readonly name: string;
      readonly slug: string;
      readonly description?: string;
      readonly provider_type: string;
      readonly credential_mode?: string;
      readonly authorization_url?: string;
      readonly token_url?: string;
      readonly revocation_url?: string;
      readonly default_scopes?: readonly string[];
      readonly client_id?: string;
      readonly client_secret?: string;
      readonly supports_pkce?: boolean;
      readonly device_code_url?: string;
      readonly device_token_url?: string;
      readonly hosted_callback_url?: string;
      readonly api_key_instructions?: string;
      readonly api_key_url?: string;
      readonly client_id_param_name?: string;
      readonly icon_url?: string;
      readonly documentation_url?: string;
    }): Promise<ProviderConfig> => {
      return api.post<ProviderConfig>("/providers", data);
    },
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ["providers"] });
    },
  });
}

export function useUpdateProvider(providerId: string) {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async (data: {
      readonly name?: string;
      readonly description?: string;
      readonly is_active?: boolean;
      readonly credential_mode?: string;
      readonly authorization_url?: string;
      readonly token_url?: string;
      readonly revocation_url?: string;
      readonly default_scopes?: readonly string[];
      readonly client_id?: string;
      readonly client_secret?: string;
      readonly supports_pkce?: boolean;
      readonly device_code_url?: string;
      readonly device_token_url?: string;
      readonly hosted_callback_url?: string;
      readonly api_key_instructions?: string;
      readonly api_key_url?: string;
      readonly client_id_param_name?: string;
      readonly icon_url?: string;
      readonly documentation_url?: string;
    }): Promise<ProviderConfig> => {
      return api.put<ProviderConfig>(`/providers/${providerId}`, data);
    },
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ["providers"] });
      void queryClient.invalidateQueries({
        queryKey: ["providers", providerId],
      });
    },
  });
}

export function useDeleteProvider() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async (id: string): Promise<void> => {
      return api.delete<void>(`/providers/${id}`);
    },
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ["providers"] });
    },
  });
}

export function useServiceRequirements(serviceId: string) {
  return useQuery({
    queryKey: ["services", serviceId, "requirements"],
    queryFn: async (): Promise<readonly ServiceProviderRequirement[]> => {
      const res = await api.get<{
        readonly requirements: readonly ServiceProviderRequirement[];
      }>(`/services/${serviceId}/requirements`);
      return res.requirements;
    },
    enabled: serviceId.length > 0,
  });
}
