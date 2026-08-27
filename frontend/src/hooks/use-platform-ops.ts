import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api } from "@/lib/api-client";
import {
  PLATFORM_OPERATION_QUERY_KEY,
  PLATFORM_OPERATION_DISCOVERY_QUERY_KEY,
  platformOperationListSchema,
  platformOperationDiscoveryListSchema,
  platformOperationSchema,
  type PlatformOperation,
  type PlatformOperationList,
  type PlatformOperationDiscoveryList,
  type UpdatePlatformOperationVariables,
} from "@/schemas/platform-ops";

export function usePlatformOperations() {
  return useQuery({
    queryKey: PLATFORM_OPERATION_QUERY_KEY,
    queryFn: async (): Promise<PlatformOperationList> => {
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
      op,
      data,
    }: UpdatePlatformOperationVariables): Promise<PlatformOperation> => {
      const response = await api.put<unknown>(
        `/admin/platform-ops/${op}`,
        data,
      );
      return platformOperationSchema.parse(response);
    },
    onSuccess: (updated) => {
      queryClient.setQueryData<PlatformOperationList>(
        PLATFORM_OPERATION_QUERY_KEY,
        (current) => {
          if (!current) return current;
          return {
            operations: current.operations.map((operation) =>
              operation.op === updated.op ? updated : operation,
            ),
          };
        },
      );
    },
  });
}
