import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api } from "@/lib/api-client";
import type { ConcurrencyResponse } from "@/schemas/service-concurrency";

export function useServiceConcurrency(serviceId: string) {
  return useQuery({
    queryKey: ["service-concurrency", serviceId],
    queryFn: () => api.get<ConcurrencyResponse>(`/services/${serviceId}/concurrency`),
  });
}
export function useUpdateServiceConcurrency(serviceId: string) {
  const client = useQueryClient();
  return useMutation({
    mutationFn: (input: ConcurrencyResponse) => api.put<ConcurrencyResponse>(`/services/${serviceId}/concurrency`, input),
    onSuccess: (data) => { client.setQueryData(["service-concurrency", serviceId], data); },
  });
}
