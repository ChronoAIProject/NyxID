import { useRef } from "react";
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
  const pendingCapability = useRef(capability);
  return useQuery({
    queryKey: ["app-connect-links", subject, id],
    queryFn: async () => {
      const capability = pendingCapability.current;
      if (capability) {
        try {
          return appConnectLinkSchema.parse(
            await api.post(`/app-connect-links/${id}/redeem`, { capability }),
          );
        } catch (error) {
          // Another tab may already have redeemed it. An authenticated read still
          // requires the same bound subject and a redeemed association.
          if (!(error instanceof ApiError) || error.errorCode !== 12101)
            throw error;
        } finally {
          // Refetches use the subject association even before the page effect
          // has scrubbed the fragment and session-storage handoff.
          pendingCapability.current = null;
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
    refetchOnWindowFocus: (query) =>
      query.state.data?.items.some((item) => item.readiness === "disabled")
        ? "always"
        : false,
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

// Consent reads the subject-bound session for display. The signed consent_request
// remains the only authority accepted by the decision endpoint.
export function useAppConnectConsent(id: string | null) {
  return useQuery({
    queryKey: ["app-connect-consent", id],
    queryFn: async () =>
      appConnectLinkSchema.parse(
        await api.get(`/app-connect-links/${encodeURIComponent(id!)}`),
      ),
    enabled: !!id,
    retry: false,
    staleTime: Infinity,
    refetchOnWindowFocus: false,
    refetchOnReconnect: false,
  });
}
