import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api, ApiError } from "@/lib/api-client";
import { useAuthStore } from "@/stores/auth-store";
import {
  servicePreferenceResponseSchema,
  type ServicePreferenceRequest,
} from "@/schemas/service-preference";

export function useServicePreference() {
  const identity = useAuthStore((state) => state.user?.id);
  const query = useQuery({
    queryKey: ["service-preference", identity],
    queryFn: async () => {
      try {
        return servicePreferenceResponseSchema.parse(
          await api.get("/service-preferences"),
        );
      } catch (error) {
        if (error instanceof ApiError && error.status === 404) return null;
        throw error;
      }
    },
    staleTime: 0,
    refetchOnMount: "always",
    retry: false,
  });
  return { ...query, data: query.isError ? undefined : query.data };
}

export function useSaveServicePreference() {
  const client = useQueryClient();
  const identity = useAuthStore((state) => state.user?.id);
  const mutation = useMutation({
    mutationFn: async (body: ServicePreferenceRequest) => {
      if (!identity || useAuthStore.getState().user?.id !== identity)
        throw new Error("Account changed before saving preference order");
      return servicePreferenceResponseSchema.parse(
        await api.put("/service-preferences", body),
      );
    },
    onSuccess: async () => {
      await Promise.all([
        client.invalidateQueries({
          queryKey: ["service-preference", identity],
        }),
        client.invalidateQueries({ queryKey: ["keys"] }),
      ]);
    },
  });
  return {
    ...mutation,
    mutateAsync: async (body: ServicePreferenceRequest) => {
      if (!identity || useAuthStore.getState().user?.id !== identity)
        throw new Error("Account changed before saving preference order");
      return mutation.mutateAsync(body);
    },
  };
}
