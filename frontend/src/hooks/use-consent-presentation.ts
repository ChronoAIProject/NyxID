import { useQuery } from "@tanstack/react-query";
import { consentPresentationSchema } from "@/schemas/oauth-consent";

export function useConsentPresentation(token: string) {
  return useQuery({
    queryKey: ["consent-presentation", token],
    queryFn: async ({ signal }) => {
      const response = await fetch(
        `/oauth/consent-presentation?consent_request=${encodeURIComponent(token)}`,
        {
          signal,
          credentials: "include",
          cache: "no-store",
          referrerPolicy: "no-referrer",
        },
      );
      if (!response.ok)
        throw new Error(
          "This consent request is unavailable. Restart sign-in from the app.",
        );
      return consentPresentationSchema.parse(await response.json());
    },
    enabled: !!token,
    retry: false,
    staleTime: Infinity,
    gcTime: 0,
    refetchOnWindowFocus: false,
    refetchOnReconnect: false,
  });
}
