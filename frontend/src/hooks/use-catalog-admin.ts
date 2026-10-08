import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api } from "@/lib/api-client";
import type { DownstreamService } from "@/types/api";
import type { PublicationState } from "@/schemas/tools";
export type SpecOverlay = {
  service_id: string;
  document: Record<string, unknown>;
  previous_document: Record<string, unknown> | null;
  sha256: string;
  revision: number;
  source: { kind: string; reference: string; version: string | null } | null;
  operations_synced?: number;
  operations_added?: number;
  operations_changed?: number;
};
export function useSpecOverlay(serviceId: string) {
  return useQuery({
    queryKey: ["services", serviceId, "overlay"],
    queryFn: () => api.get<SpecOverlay>(`/services/${serviceId}/spec-overlay`),
    retry: false,
    enabled: !!serviceId,
  });
}
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
export function useImportOverlay() {
  const client = useQueryClient();
  return useMutation({
    mutationFn: ({
      serviceId,
      document,
      source,
    }: {
      serviceId: string;
      document: Record<string, unknown>;
      source: { kind: string; reference: string; version: string | null };
    }) =>
      api.put<SpecOverlay>(`/services/${serviceId}/spec-overlay`, {
        document,
        source,
      }),
    onSuccess: () => {
      void client.invalidateQueries({ queryKey: ["services"] });
      void client.invalidateQueries({ queryKey: ["tools"] });
    },
  });
}
