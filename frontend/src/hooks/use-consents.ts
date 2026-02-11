import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api } from "@/lib/api-client";
import type { ConsentListResponse, ConsentRevokeResponse } from "@/types/rbac";

export function useMyConsents() {
  return useQuery({
    queryKey: ["consents", "me"],
    queryFn: async (): Promise<ConsentListResponse> => {
      return api.get<ConsentListResponse>("/users/me/consents");
    },
  });
}

export function useRevokeConsent() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async (clientId: string): Promise<ConsentRevokeResponse> => {
      return api.delete<ConsentRevokeResponse>(
        `/users/me/consents/${clientId}`,
      );
    },
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ["consents", "me"] });
    },
  });
}
