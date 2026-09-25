import { changedFields } from "@/lib/form-changes";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api } from "@/lib/api-client";
import type { ServiceEndpoint, DiscoverEndpointsResponse } from "@/types/api";
import type { CreateEndpointFormData } from "@/schemas/endpoints";

import {
  formToPayload,
  type CreateEndpointPayload,
} from "@/lib/endpoint-changes";

export function useEndpoints(serviceId: string) {
  return useQuery({
    queryKey: ["services", serviceId, "endpoints"],
    queryFn: async (): Promise<readonly ServiceEndpoint[]> => {
      const res = await api.get<{
        readonly endpoints: readonly ServiceEndpoint[];
      }>(`/services/${serviceId}/endpoints`);
      return res.endpoints;
    },
    enabled: serviceId.length > 0,
  });
}

export function useCreateEndpoint() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async ({
      serviceId,
      data,
    }: {
      readonly serviceId: string;
      readonly data: CreateEndpointFormData;
    }): Promise<ServiceEndpoint> => {
      return api.post<ServiceEndpoint>(
        `/services/${serviceId}/endpoints`,
        formToPayload(data),
      );
    },
    onSuccess: (_data, variables) => {
      void queryClient.invalidateQueries({
        queryKey: ["services", variables.serviceId, "endpoints"],
      });
    },
  });
}

export function useUpdateEndpoint() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async ({
      serviceId,
      endpointId,
      data,
      before,
      patch,
    }: {
      readonly serviceId: string;
      readonly endpointId: string;
      readonly before?: ServiceEndpoint;
      readonly patch?: Partial<CreateEndpointPayload>;
      readonly data: CreateEndpointFormData;
    }): Promise<void> => {
      return api.put<void>(
        `/services/${serviceId}/endpoints/${endpointId}`,
        patch ??
          (before
            ? changedFields(
                {
                  name: before.name,
                  description: before.description,
                  method: before.method,
                  path: before.path,
                  parameters: before.parameters,
                  request_body_schema: before.request_body_schema,
                  response_description: before.response_description,
                },
                formToPayload(data),
              )
            : formToPayload(data)),
      );
    },
    onSuccess: (_data, variables) => {
      void queryClient.invalidateQueries({
        queryKey: ["services", variables.serviceId, "endpoints"],
      });
    },
  });
}

export function useDeleteEndpoint() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async ({
      serviceId,
      endpointId,
    }: {
      readonly serviceId: string;
      readonly endpointId: string;
    }): Promise<void> => {
      return api.delete<void>(`/services/${serviceId}/endpoints/${endpointId}`);
    },
    onSuccess: (_data, variables) => {
      void queryClient.invalidateQueries({
        queryKey: ["services", variables.serviceId, "endpoints"],
      });
    },
  });
}

export function useDiscoverEndpoints() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async (
      serviceId: string,
    ): Promise<DiscoverEndpointsResponse> => {
      return api.post<DiscoverEndpointsResponse>(
        `/services/${serviceId}/discover-endpoints`,
      );
    },
    onSuccess: (_data, serviceId) => {
      void queryClient.invalidateQueries({
        queryKey: ["services", serviceId, "endpoints"],
      });
    },
  });
}
