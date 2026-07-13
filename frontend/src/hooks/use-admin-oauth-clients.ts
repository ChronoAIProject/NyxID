import {
  keepPreviousData,
  useMutation,
  useQuery,
  useQueryClient,
} from "@tanstack/react-query";
import { api } from "@/lib/api-client";
import type {
  AdminOAuthClient,
  AdminOAuthClientListParams,
  AdminOAuthClientListResponse,
  BrokerSettingsResponse,
  UpdateAdminOAuthClientRequest,
  UpdateBrokerSettingsRequest,
} from "@/types/admin";

export function useAdminOAuthClients(params: AdminOAuthClientListParams) {
  return useQuery({
    queryKey: ["admin", "oauth-clients", params],
    queryFn: async (): Promise<AdminOAuthClientListResponse> => {
      const query = new URLSearchParams({
        page: String(params.page),
        per_page: String(params.per_page),
        sort: params.sort,
      });
      if (params.search) query.set("search", params.search);
      if (params.search_filters) {
        query.set("search_filters", params.search_filters);
      }
      if (params.client_type) query.set("client_type", params.client_type);
      if (params.creator_type) query.set("creator_type", params.creator_type);
      if (params.broker) query.set("broker", params.broker);
      if (params.is_active !== undefined) {
        query.set("is_active", String(params.is_active));
      }
      if (params.scope) query.set("scope", params.scope);
      if (params.created_dates) {
        query.set("created_dates", params.created_dates);
      }
      if (params.created_from) query.set("created_from", params.created_from);
      if (params.created_to) query.set("created_to", params.created_to);
      return api.get<AdminOAuthClientListResponse>(
        `/admin/oauth-clients?${query.toString()}`,
      );
    },
    placeholderData: keepPreviousData,
    staleTime: 0,
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
