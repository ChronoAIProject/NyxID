import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api } from "@/lib/api-client";
import {
  manifestSchema,
  manifestsResponseSchema,
  rolloutSchema,
  type PublishManifest,
} from "@/schemas/app-requirements";

export function useAppRequirementManifests(clientId: string) {
  return useQuery({
    queryKey: ["developer", "oauth-clients", clientId, "requirements"],
    queryFn: async () =>
      manifestsResponseSchema.parse(
        await api.get(`/developer/oauth-clients/${clientId}/requirements`),
      ),
  });
}

export function usePublishAppRequirements(clientId: string) {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: async (input: PublishManifest) =>
      manifestSchema.parse(
        await api.post(
          `/developer/oauth-clients/${clientId}/requirements`,
          input,
        ),
      ),
    onSuccess: () => {
      void queryClient.invalidateQueries({
        queryKey: ["developer", "oauth-clients"],
      });
    },
  });
}

export function useAppConnectRollout(enabled: boolean) {
  return useQuery({
    queryKey: ["admin", "settings", "app-connect"],
    queryFn: async () =>
      rolloutSchema.parse(await api.get("/admin/settings/app-connect")),
    enabled,
  });
}

export function useUpdateAppConnectRollout() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: async (rollout: "disabled" | "allowlist" | null) =>
      rolloutSchema.parse(
        await api.patch("/admin/settings/app-connect", { rollout }),
      ),
    onSuccess: () => {
      void queryClient.invalidateQueries({
        queryKey: ["admin", "settings", "app-connect"],
      });
    },
  });
}

export function useUpdateAppConnectCapability() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({
      clientId,
      enabled,
    }: {
      clientId: string;
      enabled: boolean;
    }) =>
      api.patch(`/admin/oauth-clients/${clientId}/app-connect-capability`, {
        enabled,
      }),
    onSuccess: () => {
      void queryClient.invalidateQueries({
        queryKey: ["admin", "oauth-clients"],
      });
      void queryClient.invalidateQueries({
        queryKey: ["developer", "oauth-clients"],
      });
    },
  });
}
