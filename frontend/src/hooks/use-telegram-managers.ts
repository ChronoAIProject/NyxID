import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { z } from "zod";
import { apiClient } from "@/lib/api-client";
import { useAuthStore } from "@/stores/auth-store";
import { canAdminWrite } from "@/types/api";
import { telegramManagerSchema } from "@/schemas/telegram-managers";
function guard(actor?: string) {
  return () => {
    const user = useAuthStore.getState().user;
    if (!actor || user?.id !== actor || !canAdminWrite(user))
      throw new Error(
        "Your account or admin access changed. Reload before continuing.",
      );
  };
}
export function useTelegramManagers() {
  const user = useAuthStore((s) => s.user);
  return useQuery({
    queryKey: ["telegram-managers", user?.id],
    enabled: canAdminWrite(user),
    gcTime: 0,
    queryFn: async () =>
      z.array(telegramManagerSchema).parse(
        await apiClient("/admin/telegram-managers", {
          authorityGuard: guard(user?.id),
        }),
      ),
  });
}
export function useConfigureTelegramManager() {
  const actor = useAuthStore((s) => s.user?.id);
  const client = useQueryClient();
  return useMutation({
    gcTime: 0,
    retry: false,
    mutationFn: async (token: string) =>
      telegramManagerSchema.parse(
        await apiClient("/admin/telegram-managers", {
          method: "POST",
          body: { token },
          authorityGuard: guard(actor),
        }),
      ),
    onSuccess: () =>
      client.invalidateQueries({ queryKey: ["telegram-managers", actor] }),
  });
}

export function useClearTelegramManager() {
  const actor = useAuthStore((s) => s.user?.id);
  const client = useQueryClient();
  return useMutation({
    gcTime: 0,
    retry: false,
    mutationFn: (id: string) =>
      apiClient("/admin/telegram-managers/clear", {
        method: "POST",
        body: { manager_bot_id: id },
        authorityGuard: guard(actor),
      }),
    onSuccess: () =>
      client.invalidateQueries({ queryKey: ["telegram-managers", actor] }),
  });
}
