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
    onMutate: () => client.cancelQueries({ queryKey: platformCredentialsKey }),
    onSuccess: async (saved) => {
      await client.cancelQueries({ queryKey: platformCredentialsKey });
      client.setQueryData<PlatformCredentials[]>(
        platformCredentialsKey,
        (current) =>
          current?.map((item) => (item.provider === provider ? saved : item)),
      );
      void client.invalidateQueries({ queryKey: platformCredentialsKey });
      void client.invalidateQueries({ queryKey: ["managed-onboarding"] });
      if (saved.backing?.type === "provider_oauth")
        void client.invalidateQueries({ queryKey: ["providers"] });
    },
  });
}

export function useClearPlatformCredentials(provider: string) {
  const client = useQueryClient();
  return useMutation({
    retry: false,
    gcTime: 0,
    onMutate: () => client.cancelQueries({ queryKey: platformCredentialsKey }),
    mutationFn: async (): Promise<{ saved: PlatformCredentials | null }> => {
      await api.delete(
        `/admin/platform-credentials/${encodeURIComponent(provider)}`,
      );
      // DELETE is committed. A descriptor refresh must never make it retryable.
      await client.cancelQueries({ queryKey: platformCredentialsKey });
      void client.invalidateQueries({ queryKey: ["managed-onboarding"] });
      void client.invalidateQueries({ queryKey: ["providers"] });
      void client.invalidateQueries({
        queryKey: platformCredentialsKey,
        refetchType: "none",
      });
      try {
        const providers = platformCredentialsListSchema.parse(
          await api.get("/admin/platform-credentials"),
        );
        await client.cancelQueries({ queryKey: platformCredentialsKey });
        client.setQueryData(platformCredentialsKey, providers);
        return {
          saved: providers.find((item) => item.provider === provider) ?? null,
        };
      } catch {
        return { saved: null };
      }
    },
  });
}
