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
