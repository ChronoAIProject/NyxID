import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api } from "@/lib/api-client";
import type { ServiceEndpoint, DiscoverEndpointsResponse } from "@/types/api";
import type { CreateEndpointFormData } from "@/schemas/endpoints";

interface CreateEndpointPayload {
  readonly name: string;
  readonly description?: string | null;
  readonly method: string;
  readonly path: string;
  readonly parameters?: unknown | null;
  readonly request_body_schema?: unknown | null;
  readonly response_description?: string | null;
}

function formToPayload(data: CreateEndpointFormData): CreateEndpointPayload {
  return {
    name: data.name,
    description: data.description || null,
    method: data.method,
    path: data.path,
    parameters: data.parameters ? JSON.parse(data.parameters) : null,
    request_body_schema: data.request_body_schema
      ? JSON.parse(data.request_body_schema)
      : null,
    response_description: data.response_description || null,
  };
}

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
    }: {
      readonly serviceId: string;
      readonly endpointId: string;
      readonly data: CreateEndpointFormData;
    }): Promise<void> => {
      return api.put<void>(
        `/services/${serviceId}/endpoints/${endpointId}`,
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
      return api.delete<void>(
        `/services/${serviceId}/endpoints/${endpointId}`,
      );
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
