import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api } from "@/lib/api-client";
import type {
  AdminOAuthClient,
  AdminOAuthClientListResponse,
  BrokerSettingsResponse,
  UpdateAdminOAuthClientRequest,
  UpdateBrokerSettingsRequest,
} from "@/types/admin";

export function useAdminOAuthClients() {
  return useQuery({
    queryKey: ["admin", "oauth-clients"],
    queryFn: async (): Promise<AdminOAuthClientListResponse> => {
      return api.get<AdminOAuthClientListResponse>("/admin/oauth-clients");
    },
  });
}

export function useUpdateAdminOAuthClient() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async ({
      clientId,
      data,
    }: {
      readonly clientId: string;
      readonly data: UpdateAdminOAuthClientRequest;
    }): Promise<AdminOAuthClient> => {
      return api.patch<AdminOAuthClient>(
        `/admin/oauth-clients/${encodeURIComponent(clientId)}`,
        data,
      );
    },
    onSuccess: () => {
      void queryClient.invalidateQueries({
        queryKey: ["admin", "oauth-clients"],
      });
    },
  });
}

export function useBrokerSettings(enabled = true) {
  return useQuery({
    queryKey: ["admin", "settings", "broker"],
    queryFn: async (): Promise<BrokerSettingsResponse> => {
      return api.get<BrokerSettingsResponse>("/admin/settings/broker");
    },
    enabled,
  });
}

export function useUpdateBrokerSettings() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async (
      data: UpdateBrokerSettingsRequest,
    ): Promise<BrokerSettingsResponse> => {
      return api.patch<BrokerSettingsResponse>("/admin/settings/broker", data);
    },
    onSuccess: () => {
      void queryClient.invalidateQueries({
        queryKey: ["admin", "settings", "broker"],
      });
      void queryClient.invalidateQueries({
        queryKey: ["admin", "oauth-clients"],
      });
    },
  });
}
