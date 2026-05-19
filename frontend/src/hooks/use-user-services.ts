import { useQuery } from "@tanstack/react-query";
import { api } from "@/lib/api-client";
import type {
  UserServiceListResponse,
  UserServiceResponse,
} from "@/schemas/keys";

const USER_SERVICES_KEY = ["user-services"] as const;

/**
 * Fetch the union of personal and org-inherited user services.
 *
 * Each item carries a `credential_source` discriminated union so the UI can
 * group personal items vs. org-inherited ones and disable viewer-role items
 * (`credential_source.allowed === false`).
 */
export function useUserServices() {
  return useQuery({
    queryKey: USER_SERVICES_KEY,
    queryFn: async (): Promise<readonly UserServiceResponse[]> => {
      const res = await api.get<UserServiceListResponse>("/user-services");
      return res.services;
    },
  });
}
