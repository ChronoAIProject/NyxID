import { useMutation, useQuery } from "@tanstack/react-query";
import { api } from "@/lib/api-client";
import { useAuthStore } from "@/stores/auth-store";
import {
  telegramClaimPreviewSchema,
  telegramNewConfigSchema,
  telegramNewRequestSchema,
} from "@/schemas/telegram-new";

const ROOT = "/channel-bots/telegram-new";

export function useTelegramClaim(code: string, label: string, orgId?: string) {
  const actor = useAuthStore((state) => state.user?.id);
  // Secrets stay in component memory, never mutation variables or query keys.
  const preview = useMutation({
    gcTime: 0,
    retry: false,
    mutationFn: async () =>
      telegramClaimPreviewSchema.parse(
        await api.post(`${ROOT}/claims/preview`, { code }),
      ),
  });
  const redeem = useMutation({
    gcTime: 0,
    retry: false,
    mutationFn: async () =>
      telegramNewRequestSchema.parse(
        await api.post(`${ROOT}/claims/redeem`, {
          code,
          label,
          target_org_id: orgId,
        }),
      ),
  });
  const existing = useQuery({
    queryKey: ["telegram-new", actor, "claim-conflict"],
    enabled: false,
    retry: false,
    queryFn: async () => telegramNewConfigSchema.parse(await api.get(ROOT)),
  });
  const cancel = useMutation({
    mutationFn: async (id: string) => api.delete(`${ROOT}/requests/${id}`),
    onSuccess: () => existing.refetch(),
  });
  return { preview, redeem, existing, cancel };
}
