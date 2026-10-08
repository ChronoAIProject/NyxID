import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api } from "@/lib/api-client";
import type { DownstreamService } from "@/types/api";
import type { PublicationState } from "@/schemas/tools";
export function useCatalogToolMutation() {
  const client = useQueryClient();
  return useMutation({
    mutationFn: ({
      serviceId,
      body,
    }: {
      serviceId?: string;
      body: Record<string, unknown>;
    }) =>
      serviceId
        ? api.put<DownstreamService>(`/services/${serviceId}`, body)
        : api.post<DownstreamService>("/services", body),
    onSuccess: () => {
      void client.invalidateQueries({ queryKey: ["services"] });
      void client.invalidateQueries({ queryKey: ["tools"] });
    },
  });
}
export function usePublication() {
  const client = useQueryClient();
  return useMutation({
    mutationFn: ({
      serviceId,
      endpointId,
      state,
    }: {
      serviceId: string;
      endpointId: string;
      state: PublicationState;
    }) =>
      api.post(`/services/${serviceId}/endpoints/${endpointId}/publication`, {
        state,
      }),
    onSuccess: () => {
      void client.invalidateQueries({ queryKey: ["services"] });
      void client.invalidateQueries({ queryKey: ["tools"] });
    },
  });
}
