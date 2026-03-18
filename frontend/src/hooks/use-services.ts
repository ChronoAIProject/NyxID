import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { ApiError, api } from "@/lib/api-client";
import type {
  DownstreamService,
  OidcCredentials,
  RedirectUrisResponse,
  RegenerateSecretResponse,
  SshServiceConfig,
  UserServiceConnection,
} from "@/types/api";
import type { CreateServiceFormData, UpdateServiceFormData } from "@/schemas/services";

export function useServices() {
  return useQuery({
    queryKey: ["services"],
    queryFn: async (): Promise<readonly DownstreamService[]> => {
      const res = await api.get<{
        readonly services: readonly DownstreamService[];
      }>("/services");
      return res.services;
    },
  });
}

export function useService(serviceId: string) {
  return useQuery({
    queryKey: ["services", serviceId],
    queryFn: async (): Promise<DownstreamService> => {
      return api.get<DownstreamService>(`/services/${serviceId}`);
    },
    enabled: serviceId.length > 0,
  });
}

export function useCreateService() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async (
      data: CreateServiceFormData,
    ): Promise<DownstreamService> => {
      return api.post<DownstreamService>("/services", data);
    },
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ["services"] });
    },
  });
}

export function useUpdateService() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async ({
      serviceId,
      data,
    }: {
      readonly serviceId: string;
      readonly data: UpdateServiceFormData;
    }): Promise<DownstreamService> => {
      return api.put<DownstreamService>(`/services/${serviceId}`, data);
    },
    onSuccess: (_data, variables) => {
      void queryClient.invalidateQueries({ queryKey: ["services"] });
      void queryClient.invalidateQueries({
        queryKey: ["services", variables.serviceId],
      });
    },
  });
}

export function useDeleteService() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async (id: string): Promise<void> => {
      return api.delete<void>(`/services/${id}`);
    },
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ["services"] });
    },
  });
}

interface UpsertSshServiceConfigInput {
  readonly serviceId: string;
  readonly data: {
    readonly host: string;
    readonly port: number;
    readonly certificate_auth_enabled: boolean;
    readonly certificate_ttl_minutes: number;
    readonly allowed_principals: readonly string[];
  };
}

export function useSshServiceConfig(serviceId: string) {
  return useQuery({
    queryKey: ["services", serviceId, "ssh"],
    queryFn: async (): Promise<SshServiceConfig | null> => {
      try {
        return await api.get<SshServiceConfig>(`/services/${serviceId}/ssh`);
      } catch (error) {
        if (error instanceof ApiError && error.status === 404) {
          return null;
        }
        throw error;
      }
    },
    enabled: serviceId.length > 0,
    retry: false,
  });
}

export function useUpsertSshServiceConfig() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async ({
      serviceId,
      data,
    }: UpsertSshServiceConfigInput): Promise<SshServiceConfig> => {
      return api.put<SshServiceConfig>(`/services/${serviceId}/ssh`, data);
    },
    onSuccess: (_data, variables) => {
      void queryClient.invalidateQueries({
        queryKey: ["services", variables.serviceId, "ssh"],
      });
    },
  });
}

export function useDeleteSshServiceConfig() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async (serviceId: string): Promise<void> => {
      return api.delete<void>(`/services/${serviceId}/ssh`);
    },
    onSuccess: (_data, serviceId) => {
      void queryClient.invalidateQueries({
        queryKey: ["services", serviceId, "ssh"],
      });
    },
  });
}

export function useOidcCredentials(serviceId: string, enabled: boolean) {
  return useQuery({
    queryKey: ["services", serviceId, "oidc-credentials"],
    queryFn: async (): Promise<OidcCredentials> => {
      return api.get<OidcCredentials>(
        `/services/${serviceId}/oidc-credentials`,
      );
    },
    enabled: enabled && serviceId.length > 0,
    staleTime: 0,
  });
}

export function useUpdateRedirectUris() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async ({
      serviceId,
      redirectUris,
    }: {
      readonly serviceId: string;
      readonly redirectUris: readonly string[];
    }): Promise<RedirectUrisResponse> => {
      return api.put<RedirectUrisResponse>(
        `/services/${serviceId}/redirect-uris`,
        { redirect_uris: redirectUris },
      );
    },
    onSuccess: (_data, variables) => {
      void queryClient.invalidateQueries({
        queryKey: ["services", variables.serviceId, "oidc-credentials"],
      });
    },
  });
}

export function useRegenerateOidcSecret() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async (
      serviceId: string,
    ): Promise<RegenerateSecretResponse> => {
      return api.post<RegenerateSecretResponse>(
        `/services/${serviceId}/regenerate-secret`,
      );
    },
    onSuccess: (_data, serviceId) => {
      void queryClient.invalidateQueries({
        queryKey: ["services", serviceId, "oidc-credentials"],
      });
    },
  });
}

export function useConnections() {
  return useQuery({
    queryKey: ["connections"],
    queryFn: async (): Promise<readonly UserServiceConnection[]> => {
      const res = await api.get<{
        readonly connections: readonly UserServiceConnection[];
      }>("/connections");
      return res.connections;
    },
  });
}

interface ConnectServiceParams {
  readonly serviceId: string;
  readonly credential?: string;
  readonly credentialLabel?: string;
}

interface ConnectResponse {
  readonly service_id: string;
  readonly service_name: string;
  readonly connected_at: string;
}

export function useConnectService() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async (
      params: ConnectServiceParams,
    ): Promise<ConnectResponse> => {
      return api.post<ConnectResponse>(
        `/connections/${params.serviceId}`,
        {
          credential: params.credential,
          credential_label: params.credentialLabel,
        },
      );
    },
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ["connections"] });
    },
  });
}

export function useUpdateCredential() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async (params: {
      readonly serviceId: string;
      readonly credential: string;
      readonly credentialLabel?: string;
    }): Promise<void> => {
      return api.put<void>(`/connections/${params.serviceId}/credential`, {
        credential: params.credential,
        credential_label: params.credentialLabel,
      });
    },
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ["connections"] });
    },
  });
}

export function useDisconnectService() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async (serviceId: string): Promise<void> => {
      return api.delete<void>(`/connections/${serviceId}`);
    },
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ["connections"] });
    },
  });
}
