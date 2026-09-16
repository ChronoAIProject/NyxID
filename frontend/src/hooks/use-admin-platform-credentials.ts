import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api } from "@/lib/api-client";
import {
  platformCredentialsListSchema,
  platformCredentialsSchema,
} from "@/schemas/admin-platform-credentials";
import type {
  PlatformCredentials,
  PlatformCredentialsUpdate,
} from "@/types/admin";

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
    mutationFn: async (body: PlatformCredentialsUpdate) =>
      platformCredentialsSchema.parse(
        await api.patch(
          `/admin/platform-credentials/${encodeURIComponent(provider)}`,
          body,
        ),
      ),
    onSuccess: async (saved) => {
      client.setQueryData<PlatformCredentials[]>(
        platformCredentialsKey,
        (current) =>
          current?.map((item) => (item.provider === provider ? saved : item)),
      );
      await client.invalidateQueries({ queryKey: platformCredentialsKey });
      await client.invalidateQueries({ queryKey: ["managed-onboarding"] });
    },
  });
}

export function useClearPlatformCredentials(provider: string) {
  const client = useQueryClient();
  return useMutation({
    mutationFn: async () => {
      await api.delete(
        `/admin/platform-credentials/${encodeURIComponent(provider)}`,
      );
      const providers = platformCredentialsListSchema.parse(
        await api.get("/admin/platform-credentials"),
      );
      client.setQueryData(platformCredentialsKey, providers);
      const saved = providers.find((item) => item.provider === provider);
      if (!saved) throw new Error("Unable to reload provider credentials");
      return saved;
    },
    onSuccess: async () => {
      await client.invalidateQueries({ queryKey: platformCredentialsKey });
      await client.invalidateQueries({ queryKey: ["managed-onboarding"] });
    },
  });
}
