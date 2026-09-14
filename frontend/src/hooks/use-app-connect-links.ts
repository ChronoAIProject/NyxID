import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api, ApiError } from "@/lib/api-client";
import {
  appConnectChildSchema,
  appConnectLinkSchema,
  handoffResponseSchema,
  type HandoffForm,
} from "@/schemas/app-connect-links";

export function useAppConnectLink(
  id: string,
  subject: string | undefined,
  enabled: boolean,
  capability: string | null = null,
) {
  return useQuery({
    queryKey: ["app-connect-links", subject, id],
    queryFn: async () => {
      if (capability && window.location.hash) {
        try {
          return appConnectLinkSchema.parse(
            await api.post(`/app-connect-links/${id}/redeem`, { capability }),
          );
        } catch (error) {
          // Another tab may already have redeemed it. An authenticated read still
          // requires the same bound subject and a redeemed association.
          if (!(error instanceof ApiError) || error.errorCode !== 12001)
            throw error;
        }
      }
      return appConnectLinkSchema.parse(
        await api.get(`/app-connect-links/${id}`),
      );
    },
    enabled: enabled && !!subject,
    retry: false,
    staleTime: Infinity,
    refetchOnMount: "always",
    refetchOnWindowFocus: false,
    refetchOnReconnect: false,
  });
}

export function useAppConnectAction(id: string, subject: string | undefined) {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: async ({ path, body }: { path: string; body?: unknown }) =>
      appConnectLinkSchema.parse(
        await api.post(`/app-connect-links/${id}/${path}`, body ?? {}),
      ),
    onSuccess: (link) =>
      queryClient.setQueryData(["app-connect-links", subject, id], link),
  });
}

export function useAppConnectChild(id: string) {
  return useMutation({
    mutationFn: async ({
      requirementId,
      slug,
      reauthorize,
    }: {
      requirementId: string;
      slug: string;
      reauthorize: boolean;
    }) =>
      appConnectChildSchema.parse(
        await api.post(
          `/app-connect-links/${id}/items/${encodeURIComponent(requirementId)}/${reauthorize ? "reauthorize" : "connect"}`,
          { service_slug: slug },
        ),
      ),
  });
}

export function useUpdateAppHandoff(clientId: string) {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: async (input: HandoffForm) =>
      handoffResponseSchema.parse(
        await api.patch(`/developer/oauth-clients/${clientId}/handoff`, input),
      ),
    onSuccess: () => {
      void queryClient.invalidateQueries({
        queryKey: ["developer", "oauth-clients"],
      });
    },
  });
}
