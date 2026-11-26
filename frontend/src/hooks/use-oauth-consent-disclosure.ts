import { useQuery } from "@tanstack/react-query";
import { apiClient } from "@/lib/api-client";
import {
  consentDisclosureSchema,
  existingConsentAccessSchema,
  MAX_CONSENT_DISCLOSURE_SERVICES,
  type ConsentDisclosure,
} from "@/schemas/oauth-consent";
import { useAuthStore } from "@/stores/auth-store";

export function useOAuthConsentDisclosure(
  serviceIds: readonly string[],
  enabled: boolean,
) {
  const owner = useAuthStore((state) => state.user?.id);
  const ids = [...new Set(serviceIds)].sort();
  return useQuery({
    queryKey: ["oauth-consent-disclosure", owner, ids],
    enabled: enabled && Boolean(owner),
    retry: false,
    queryFn: async ({ signal }): Promise<ConsentDisclosure> => {
      const readBatch = async (batch: readonly string[]) =>
        consentDisclosureSchema.parse(
          await apiClient<unknown>("/user-services/consent-disclosure", {
            method: "POST",
            body: { service_ids: batch },
            signal,
          }),
        );

      const result = await readBatch(
        ids.slice(0, MAX_CONSENT_DISCLOSURE_SERVICES),
      );
      for (
        let start = MAX_CONSENT_DISCLOSURE_SERVICES;
        start < ids.length;
        start += MAX_CONSENT_DISCLOSURE_SERVICES
      ) {
        const batch = await readBatch(
          ids.slice(start, start + MAX_CONSENT_DISCLOSURE_SERVICES),
        );
        result.services.push(...batch.services);
      }
      return result;
    },
  });
}

export function useExistingOAuthConsent(clientId: string, enabled: boolean) {
  const owner = useAuthStore((state) => state.user?.id);
  return useQuery({
    queryKey: ["oauth-consent-existing", owner, clientId],
    enabled: enabled && Boolean(owner),
    retry: false,
    queryFn: async ({ signal }) => {
      const value = existingConsentAccessSchema.parse(
        await apiClient<unknown>(
          `/users/me/consents/${encodeURIComponent(clientId)}/authorization`,
          { signal },
        ),
      );
      return value.expires_at && Date.parse(value.expires_at) <= Date.now()
        ? null
        : value;
    },
  });
}
