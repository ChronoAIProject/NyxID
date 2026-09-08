import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api } from "@/lib/api-client";
import { platformCredentialsListSchema } from "@/schemas/admin-platform-credentials";
import type { PlatformCredentialsUpdate } from "@/types/admin";

export const platformCredentialsKey = [
  "admin",
  "platform-credentials",
] as const;

export function useAdminPlatformCredentials() {
  return useQuery({
    queryKey: platformCredentialsKey,
    gcTime: 0,
    queryFn: async () =>
      platformCredentialsListSchema.parse(
        await api.get("/admin/platform-credentials"),
      ),
  });
}

export function useUpdatePlatformCredentials(provider: string) {
  const client = useQueryClient();
  return useMutation({
    gcTime: 0,
    mutationFn: (body: PlatformCredentialsUpdate) =>
      api.patch(
        `/admin/platform-credentials/${encodeURIComponent(provider)}`,
        body,
      ),
    onSuccess: async () => {
      await client.invalidateQueries({ queryKey: platformCredentialsKey });
      await client.invalidateQueries({ queryKey: ["managed-onboarding"] });
    },
  });
}

export function useClearPlatformCredentials(provider: string) {
  const client = useQueryClient();
  return useMutation({
    mutationFn: () =>
      api.delete(`/admin/platform-credentials/${encodeURIComponent(provider)}`),
    onSuccess: async () => {
      await client.invalidateQueries({ queryKey: platformCredentialsKey });
      await client.invalidateQueries({ queryKey: ["managed-onboarding"] });
    },
  });
}
