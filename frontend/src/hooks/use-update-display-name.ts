import { useMutation, useQueryClient } from "@tanstack/react-query";
import { api } from "@/lib/api-client";
import { useAuthStore } from "@/stores/auth-store";
import type { User } from "@/types/api";

export function useUpdateDisplayName(userId: string) {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: async (values: { display_name: string }) => {
      const updated = await api.put<
        Pick<User, "id" | "email" | "display_name" | "avatar_url">
      >("/users/me", values, {
        authorityGuard: () => {
          if (useAuthStore.getState().user?.id !== userId)
            throw new Error(
              "Account changed; try again from the current account.",
            );
        },
      });
      const current = useAuthStore.getState().user;
      if (!current || current.id !== userId || updated.id !== userId)
        throw new Error("Profile response did not match the current account.");
      // PUT returns only editable fields. Keep the full profile, including consent and capabilities.
      const merged = {
        ...current,
        email: updated.email,
        display_name: updated.display_name,
        avatar_url: updated.avatar_url,
      };
      useAuthStore.getState().setUser(merged);
      queryClient.setQueryData(["user", "me"], merged);
      return updated;
    },
  });
}
