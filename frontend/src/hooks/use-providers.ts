import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api } from "@/lib/api-client";
import { telegramLoginDataSchema } from "@/schemas/providers";
import type {
  MessageResponse,
  ProviderConfig,
  ProviderListResponse,
  ProviderActionResponse,
  UserTokenListResponse,
  UserProviderToken,
  UserProviderCredentials,
  OAuthInitiateResponse,
  DeviceCodeInitiateResponse,
  DeviceCodePollRequest,
  DeviceCodePollResponse,
  ServiceProviderRequirement,
  TelegramWidgetConfig,
  TelegramLoginData,
} from "@/types/api";

interface ProviderTokenScopeOptions {
  readonly targetOrgId?: string | null;
}

interface ScopedProviderMutationInput extends ProviderTokenScopeOptions {
  readonly providerId: string;
}

function providerTokenQueryKey(targetOrgId: string | null | undefined) {
  return ["provider-tokens", targetOrgId ?? "personal"] as const;
}

function targetOrgSuffix(targetOrgId: string | null | undefined): string {
  if (!targetOrgId) return "";
  const query = new URLSearchParams({ target_org_id: targetOrgId });
  return `?${query.toString()}`;
}

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

export function useMyProviderTokens(options: ProviderTokenScopeOptions = {}) {
  const targetOrgId = options.targetOrgId ?? null;

  return useQuery({
    queryKey: providerTokenQueryKey(targetOrgId),
    queryFn: async (): Promise<readonly UserProviderToken[]> => {
      const res = await api.get<UserTokenListResponse>(
        `/providers/my-tokens${targetOrgSuffix(targetOrgId)}`,
      );
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
      gatewayUrl,
    }: {
      readonly providerId: string;
      readonly apiKey: string;
      readonly label?: string;
      readonly gatewayUrl?: string;
    }): Promise<ProviderActionResponse> => {
      return api.post<ProviderActionResponse>(
        `/providers/${providerId}/connect/api-key`,
        {
          api_key: apiKey,
          label,
          gateway_url: gatewayUrl || undefined,
        },
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
    mutationFn: async (
      input:
        | string
        | {
            readonly providerId: string;
            readonly redirectPath?: string;
            /**
             * Additional OAuth scopes to request on top of the provider's
             * `default_scopes`. Sent as a comma-separated `scope` query param;
             * the backend splits on comma/whitespace, validates, and merges.
             */
            readonly additionalScopes?: readonly string[];
            /**
             * Complete scope set from the scope picker (NyxID#917). When
             * provided (even empty), sent as `scope_override`, which REPLACES
             * the provider's default scopes server-side instead of appending —
             * so the user can drop a default. Takes precedence over
             * `additionalScopes`.
             */
            readonly scopeOverride?: readonly string[];
            /**
             * When set, initiate the OAuth flow on behalf of the given org.
             * The resulting token is stored under the org's user_id so every
             * org member can proxy through it. Caller must be an org admin.
             */
            readonly targetOrgId?: string;
            /**
             * Multi-connection: the freshly-minted placeholder `UserService`
             * id from a preceding `POST /keys`. The backend's OAuth-state
             * insert reads the placeholder's `UserApiKey.connection_id` from
             * this id and stamps it onto `OAuthState`, so the eventual
             * callback writes the tokens straight onto that `UserApiKey`
             * (instead of the legacy `user_provider_tokens` row). Without
             * this, a multi-connection placeholder stays `pending_auth`
             * forever and the token aliases onto the legacy single-tenant
             * path — defeating the whole point of `connection_id`. Omit
             * for legacy add-flows that don't pre-create a placeholder.
             */
            readonly keyId?: string;
          },
    ): Promise<OAuthInitiateResponse> => {
      const params =
        typeof input === "string" ? { providerId: input } : input;
      const query = new URLSearchParams();
      if (params.redirectPath) {
        query.set("redirect_path", params.redirectPath);
      }
      // `scope_override` (full set) wins over additive `scope`. Sent whenever
      // defined — including an empty array, which the backend reads as "user
      // cleared all scopes" and omits the param so the provider applies its
      // own minimum.
      if (params.scopeOverride !== undefined) {
        query.set("scope_override", params.scopeOverride.join(","));
      } else if (params.additionalScopes && params.additionalScopes.length > 0) {
        query.set("scope", params.additionalScopes.join(","));
      }
      if (params.targetOrgId) {
        query.set("target_org_id", params.targetOrgId);
      }
      if (params.keyId) {
        query.set("key_id", params.keyId);
      }
      const queryString = query.toString();
      const suffix = queryString ? `?${queryString}` : "";
      return api.get<OAuthInitiateResponse>(
        `/providers/${params.providerId}/connect/oauth${suffix}`,
      );
    },
  });
}

export function useInitiateDeviceCode() {
  return useMutation({
    mutationFn: async (
      input:
        | string
        | {
            readonly providerId: string;
            readonly additionalScopes?: readonly string[];
            /** Same contract as `useInitiateOAuth`'s `scopeOverride`. */
            readonly scopeOverride?: readonly string[];
            /** Same contract as `useInitiateOAuth`'s `targetOrgId`. */
            readonly targetOrgId?: string;
            /** Same contract as `useInitiateOAuth`'s `keyId`. */
            readonly keyId?: string;
          },
    ): Promise<DeviceCodeInitiateResponse> => {
      const params =
        typeof input === "string" ? { providerId: input } : input;
      const query = new URLSearchParams();
      if (params.scopeOverride !== undefined) {
        query.set("scope_override", params.scopeOverride.join(","));
      } else if (params.additionalScopes && params.additionalScopes.length > 0) {
        query.set("scope", params.additionalScopes.join(","));
      }
      if (params.targetOrgId) {
        query.set("target_org_id", params.targetOrgId);
      }
      if (params.keyId) {
        query.set("key_id", params.keyId);
      }
      const queryString = query.toString();
      const suffix = queryString ? `?${queryString}` : "";
      return api.post<DeviceCodeInitiateResponse>(
        `/providers/${params.providerId}/connect/device-code/initiate${suffix}`,
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
    mutationFn: async ({
      providerId,
      targetOrgId,
    }: ScopedProviderMutationInput): Promise<ProviderActionResponse> => {
      return api.delete<ProviderActionResponse>(
        `/providers/${providerId}/disconnect${targetOrgSuffix(targetOrgId)}`,
      );
    },
    onSuccess: (_data, variables) => {
      void queryClient.invalidateQueries({
        queryKey: providerTokenQueryKey(variables.targetOrgId),
      });
      void queryClient.invalidateQueries({ queryKey: ["providers"] });
      void queryClient.invalidateQueries({ queryKey: ["llm-status"] });
    },
  });
}

export function useRefreshProviderToken() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async ({
      providerId,
      targetOrgId,
    }: ScopedProviderMutationInput): Promise<ProviderActionResponse> => {
      return api.post<ProviderActionResponse>(
        `/providers/${providerId}/refresh${targetOrgSuffix(targetOrgId)}`,
      );
    },
    onSuccess: (_data, variables) => {
      void queryClient.invalidateQueries({
        queryKey: providerTokenQueryKey(variables.targetOrgId),
      });
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
    }): Promise<ProviderActionResponse> => {
      const parsedData = telegramLoginDataSchema.parse(data);
      return api.post<ProviderActionResponse>(
        `/providers/${providerId}/connect/telegram/callback`,
        parsedData,
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
    mutationFn: async (providerId: string): Promise<MessageResponse> => {
      return api.delete<MessageResponse>(
        `/providers/${providerId}/credentials`,
      );
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
      readonly device_verification_url?: string;
      readonly hosted_callback_url?: string;
      readonly api_key_instructions?: string;
      readonly api_key_url?: string;
      readonly extra_auth_params?: Readonly<Record<string, string>>;
      readonly device_code_format?: "rfc8628" | "openai";
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
      readonly device_verification_url?: string;
      readonly hosted_callback_url?: string;
      readonly api_key_instructions?: string;
      readonly api_key_url?: string;
      readonly extra_auth_params?: Readonly<Record<string, string>>;
      readonly device_code_format?: "rfc8628" | "openai";
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
    mutationFn: async (id: string): Promise<MessageResponse> => {
      return api.delete<MessageResponse>(`/providers/${id}`);
    },
    onSuccess: (_data, id) => {
      void queryClient.invalidateQueries({ queryKey: ["providers"] });
      void queryClient.invalidateQueries({
        queryKey: ["providers", id],
      });
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
