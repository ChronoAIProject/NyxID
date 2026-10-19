import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api } from "@/lib/api-client";

export interface KeyReadGrant {
  readonly service_account_id: string;
  readonly user_service_ids: readonly string[];
  readonly issued_by: string;
  readonly issued_at: string;
  readonly expires_at: string | null;
}

export function useSaKeyReadGrant(saId: string) {
  return useQuery({
    queryKey: ["admin", "service-accounts", saId, "key-read-grant"],
    queryFn: () => api.get<KeyReadGrant | null>(`/admin/service-accounts/${saId}/key-read-grant`),
  });
}

export function useSaveSaKeyReadGrant(saId: string) {
  const client = useQueryClient();
  return useMutation({
    mutationFn: (data: { readonly user_service_ids: readonly string[]; readonly expires_at?: string }) =>
      api.put<KeyReadGrant>(`/admin/service-accounts/${saId}/key-read-grant`, data),
    onSuccess: (data) => client.setQueryData(["admin", "service-accounts", saId, "key-read-grant"], data),
  });
}

export function useRevokeSaKeyReadGrant(saId: string) {
  const client = useQueryClient();
  return useMutation({
    mutationFn: () => api.delete(`/admin/service-accounts/${saId}/key-read-grant`),
    onSuccess: () => client.setQueryData(["admin", "service-accounts", saId, "key-read-grant"], null),
  });
}
