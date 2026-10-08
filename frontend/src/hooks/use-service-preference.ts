import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api, apiClient, ApiError } from "@/lib/api-client";
import { useAuthStore } from "@/stores/auth-store";
import {
  releaseHiddenPreferenceRequestSchema,
  serviceGroupKeySchema,
  servicePreferenceGroupRequestSchema,
  servicePreferenceResponseSchema,
  type ServicePreferenceGroupRequest,
} from "@/schemas/service-preference";

export const SERVICE_ORDER_UNAVAILABLE = "unavailable" as const;
export function useServicePreference() {
  const identity = useAuthStore((state) => state.user?.id);
  const query = useQuery({
    queryKey: ["service-preference", identity],
    enabled: Boolean(identity),
    queryFn: async ({ queryKey }) => {
      assertIdentity(queryKey[1]);
      try {
        return servicePreferenceResponseSchema.parse(
          await api.get("/service-preferences", {
            authorityGuard: () => assertIdentity(queryKey[1]),
          }),
        );
      } catch (error) {
        if (error instanceof ApiError && error.status === 404)
          return SERVICE_ORDER_UNAVAILABLE;
        throw error;
      }
    },
    staleTime: 0,
    refetchOnMount: "always",
    retry: false,
  });
  return { ...query, data: query.isError ? undefined : query.data };
}

function assertIdentity(actor: string | undefined): asserts actor is string {
  if (!actor || useAuthStore.getState().user?.id !== actor)
    throw new Error("Account changed before saving agent order");
}
export function useSaveServiceGroupOrder() {
  const client = useQueryClient();
  const identity = useAuthStore((state) => state.user?.id);
  const invalidate = async (actor: string) => {
    await client.invalidateQueries({
      queryKey: ["service-preference", actor],
      refetchType:
        useAuthStore.getState().user?.id === actor ? "active" : "none",
    });
    if (useAuthStore.getState().user?.id === actor)
      await client.invalidateQueries({ queryKey: ["keys", "list", actor] });
  };
  const save = useMutation({
    mutationFn: async ({
      actor,
      group,
      body,
    }: {
      actor: string;
      group: string;
      body: ServicePreferenceGroupRequest;
    }) => {
      const parsedGroup = serviceGroupKeySchema.parse(group);
      const parsedBody = servicePreferenceGroupRequestSchema.parse(body);
      assertIdentity(actor);
      return servicePreferenceResponseSchema.parse(
        await api.put(
          `/service-preferences/groups/${encodeURIComponent(parsedGroup)}`,
          parsedBody,
          { authorityGuard: () => assertIdentity(actor) },
        ),
      );
    },
    onSuccess: (_result, variables) => invalidate(variables.actor),
  });
  const release = useMutation({
    mutationFn: async ({
      actor,
      version,
    }: {
      actor: string;
      version: number;
    }) => {
      const body = releaseHiddenPreferenceRequestSchema.parse({
        expected_version: version,
      });
      assertIdentity(actor);
      return servicePreferenceResponseSchema.parse(
        await apiClient("/service-preferences/hidden", {
          method: "DELETE",
          body,
          authorityGuard: () => assertIdentity(actor),
        }),
      );
    },
    onSuccess: (_result, variables) => invalidate(variables.actor),
  });
  return {
    save: async (group: string, body: ServicePreferenceGroupRequest) => {
      assertIdentity(identity);
      return save.mutateAsync({ actor: identity, group, body });
    },
    release: async (version: number) => {
      assertIdentity(identity);
      return release.mutateAsync({ actor: identity, version });
    },
    isPending:
      (save.isPending && save.variables?.actor === identity) ||
      (release.isPending && release.variables?.actor === identity),
  };
}
