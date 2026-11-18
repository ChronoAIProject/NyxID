import { useMutation, useQueryClient } from "@tanstack/react-query";
import { captureAccountProfileRead } from "@/lib/assistant/identity";
import { apiClient } from "@/lib/api-client";
import { securityPreferencesSchema } from "@/schemas/security";
import { useAuthStore } from "@/stores/auth-store";
import type { SecurityPreferences, User } from "@/types/api";

export function useSaveSecurityPreferences() {
  const client = useQueryClient();
  return useMutation({
    retry: false,
    mutationFn: async ({ accountId, ...input }: SecurityPreferences & { accountId: string }) => {
      if (useAuthStore.getState().user?.id !== accountId) throw new Error("Account changed. Try again.");
      const profile = captureAccountProfileRead();
      const saved = await apiClient<SecurityPreferences>("/users/me/security", {
        method: "PUT", body: securityPreferencesSchema.parse(input), preserveSessionOn401: true,
        authorityGuard: () => {
          if (useAuthStore.getState().user?.id !== accountId) throw new Error("Account changed. Try again.");
        },
      });
      return { accountId, profile, saved: securityPreferencesSchema.parse(saved) };
    },
    onSuccess: ({ accountId, profile, saved }) => {
      const user = useAuthStore.getState().user;
      if (user?.id !== accountId || !profile.isCurrent()) return;
      const updated: User = {
        ...user,
        profile_config: { ...user.profile_config, onboarding: user.profile_config?.onboarding ?? { ai_services_completed_at: null }, security: saved },
      };
      useAuthStore.getState().setUser(updated);
      client.setQueryData(["user", "me"], updated);
    },
  });
}
