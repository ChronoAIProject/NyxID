import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api } from "@/lib/api-client";
import {
  PLATFORM_OPERATION_DISCOVERY_QUERY_KEY,
  PLATFORM_OPERATION_QUERY_KEY,
  adminPlatformOperationSchema,
  adminPlatformProviderListSchema,
  adminPlatformProviderSchema,
  platformOperationDiscoveryListSchema,
  platformOperationListSchema,
  type AdminPlatformOperation,
  type AdminPlatformOperationList,
  type AdminPlatformProvider,
  type AdminPlatformProviderList,
  type PlatformCredentialWrite,
  type PlatformOperationDiscoveryList,
  type UpdateAdminPlatformOperation,
} from "@/schemas/platform-ops";

export const PLATFORM_PROVIDER_QUERY_KEY = [
  "admin",
  "platform-providers",
] as const;

function replaceOperation(
  current: AdminPlatformOperationList | undefined,
  updated: AdminPlatformOperation,
): AdminPlatformOperationList | undefined {
  if (!current) return current;
  return {
    operations: current.operations.map((operation) =>
      operation.operation_id === updated.operation_id ? updated : operation,
    ),
  };
}

function replaceProvider(
  current: AdminPlatformProviderList | undefined,
  updated: AdminPlatformProvider,
): AdminPlatformProviderList | undefined {
  if (!current) return current;
  return {
    providers: current.providers.map((provider) =>
      provider.catalog_service_id === updated.catalog_service_id
        ? updated
        : provider,
    ),
  };
}

export function usePlatformOperations() {
  return useQuery({
    queryKey: PLATFORM_OPERATION_QUERY_KEY,
    queryFn: async (): Promise<AdminPlatformOperationList> => {
      const response = await api.get<unknown>("/admin/platform-ops");
      return platformOperationListSchema.parse(response);
    },
  });
}

export function usePlatformOperationDiscovery(enabled = true) {
  return useQuery({
    queryKey: PLATFORM_OPERATION_DISCOVERY_QUERY_KEY,
    enabled,
    queryFn: async (): Promise<PlatformOperationDiscoveryList> => {
      const response = await api.get<unknown>("/platform-ops");
      return platformOperationDiscoveryListSchema.parse(response);
    },
  });
}

export function useUpdatePlatformOperation() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: async ({
      operationId,
      data,
    }: {
      readonly operationId: string;
      readonly data: UpdateAdminPlatformOperation;
    }): Promise<AdminPlatformOperation> => {
      const response = await api.put<unknown>(
        `/admin/platform-ops/${operationId}`,
        data,
      );
      return adminPlatformOperationSchema.parse(response);
    },
    onSuccess: (updated) => {
      queryClient.setQueryData<AdminPlatformOperationList>(
        PLATFORM_OPERATION_QUERY_KEY,
        (current) => replaceOperation(current, updated),
      );
      void queryClient.invalidateQueries({
        queryKey: PLATFORM_OPERATION_DISCOVERY_QUERY_KEY,
      });
    },
  });
}

export function usePlatformProviders() {
  return useQuery({
    queryKey: PLATFORM_PROVIDER_QUERY_KEY,
    queryFn: async (): Promise<AdminPlatformProviderList> => {
      const response = await api.get<unknown>("/admin/platform-providers");
      return adminPlatformProviderListSchema.parse(response);
    },
  });
}

function useProviderMutation(
  request: (providerId: string) => Promise<unknown>,
) {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: async (providerId: string): Promise<AdminPlatformProvider> =>
      adminPlatformProviderSchema.parse(await request(providerId)),
    onSuccess: (updated) => {
      queryClient.setQueryData<AdminPlatformProviderList>(
        PLATFORM_PROVIDER_QUERY_KEY,
        (current) => replaceProvider(current, updated),
      );
      void queryClient.invalidateQueries({
        queryKey: PLATFORM_OPERATION_QUERY_KEY,
      });
      void queryClient.invalidateQueries({
        queryKey: PLATFORM_OPERATION_DISCOVERY_QUERY_KEY,
      });
    },
  });
}

export function usePromotePlatformProvider() {
  return useProviderMutation((providerId) =>
    api.put(`/admin/platform-providers/${providerId}`, {
      vendor_terms_accepted: true,
    }),
  );
}

export function useDemotePlatformProvider() {
  return useProviderMutation((providerId) =>
    api.delete(`/admin/platform-providers/${providerId}`),
  );
}

export function useDeletePlatformCredential() {
  return useProviderMutation((providerId) =>
    api.delete(`/admin/platform-providers/${providerId}/credential`),
  );
}

export function useSetPlatformCredential() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: async ({
      providerId,
      data,
    }: {
      readonly providerId: string;
      readonly data: PlatformCredentialWrite;
    }): Promise<AdminPlatformProvider> => {
      const response = await api.put<unknown>(
        `/admin/platform-providers/${providerId}/credential`,
        data,
      );
      return adminPlatformProviderSchema.parse(response);
    },
    onSuccess: (updated) => {
      queryClient.setQueryData<AdminPlatformProviderList>(
        PLATFORM_PROVIDER_QUERY_KEY,
        (current) => replaceProvider(current, updated),
      );
      void queryClient.invalidateQueries({
        queryKey: PLATFORM_OPERATION_QUERY_KEY,
      });
      void queryClient.invalidateQueries({
        queryKey: PLATFORM_OPERATION_DISCOVERY_QUERY_KEY,
      });
    },
  });
}
