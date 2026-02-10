import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api } from "@/lib/api-client";
import type {
  DownstreamService,
  OidcCredentials,
  RedirectUrisResponse,
  RegenerateSecretResponse,
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

export function useConnectService() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async (serviceId: string): Promise<UserServiceConnection> => {
      return api.post<UserServiceConnection>(`/connections/${serviceId}`);
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
