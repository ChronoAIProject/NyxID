import {
  useMutation,
  useQuery,
  useQueryClient,
  type QueryClient,
} from "@tanstack/react-query";
import { api } from "@/lib/api-client";
import type {
  AnonymousEndpointListResponse,
  AnonymousEndpointRule,
} from "@/types/api";
import type {
  AnonymousEndpointRuleFormData,
  AnonymousEndpointUpdateData,
} from "@/schemas/anonymous-endpoints";

export function useAnonymousEndpoints(serviceId: string) {
  return useQuery({
    queryKey: ["services", serviceId, "anonymous-endpoints"],
    queryFn: async (): Promise<readonly AnonymousEndpointRule[]> => {
      const response = await api.get<AnonymousEndpointListResponse>(
        `/services/${serviceId}/anonymous-endpoints`,
      );
      return response.endpoints;
    },
    enabled: serviceId.length > 0,
  });
}

export function useCreateAnonymousEndpoint(serviceId: string) {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: async (
      data: AnonymousEndpointRuleFormData,
    ): Promise<AnonymousEndpointRule> => {
      return api.post<AnonymousEndpointRule>(
        `/services/${serviceId}/anonymous-endpoints`,
        data,
      );
    },
    onSuccess: () => invalidateAnonymousEndpointQueries(queryClient, serviceId),
  });
}

export function useUpdateAnonymousEndpoint(serviceId: string) {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: async ({
      ruleId,
      data,
    }: {
      readonly ruleId: string;
      readonly data: AnonymousEndpointUpdateData;
    }): Promise<AnonymousEndpointRule> => {
      return api.put<AnonymousEndpointRule>(
        `/services/${serviceId}/anonymous-endpoints/${ruleId}`,
        data,
      );
    },
    onSuccess: () => invalidateAnonymousEndpointQueries(queryClient, serviceId),
  });
}

export function useDeleteAnonymousEndpoint(serviceId: string) {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: async (
      ruleId: string,
    ): Promise<AnonymousEndpointListResponse> => {
      return api.delete<AnonymousEndpointListResponse>(
        `/services/${serviceId}/anonymous-endpoints/${ruleId}`,
      );
    },
    onSuccess: () => invalidateAnonymousEndpointQueries(queryClient, serviceId),
  });
}

function invalidateAnonymousEndpointQueries(
  queryClient: QueryClient,
  serviceId: string,
) {
  void queryClient.invalidateQueries({
    queryKey: ["services", serviceId, "anonymous-endpoints"],
  });
  void queryClient.invalidateQueries({ queryKey: ["services", serviceId] });
  void queryClient.invalidateQueries({ queryKey: ["services"] });
}
