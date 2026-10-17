import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { apiClient } from "@/lib/api-client";
import { useAuthStore } from "@/stores/auth-store";
import { canAdminWrite } from "@/types/api";
import {
  nativeConversionResponseSchema,
  nativeLifecycleReviewSchema,
  type NativeLifecycleReview,
  nativeSetupRecoverySchema,
  nativeTransportsSchema,
  type NativeSetupRecovery,
  type NativeTransport,
} from "@/schemas/native-platform-entry";
function guard(actor: string | undefined) {
  return () => {
    const user = useAuthStore.getState().user;
    if (!actor || actor !== user?.id || !canAdminWrite(user))
      throw new Error(
        "Your administrator access changed. Reload before continuing.",
      );
  };
}
export function useNativeEntryTransports() {
  const user = useAuthStore((s) => s.user);
  return useQuery({
    queryKey: ["native-entry-transports", user?.id],
    enabled: canAdminWrite(user),
    gcTime: 0,
    queryFn: async () =>
      nativeTransportsSchema.parse(
        await apiClient("/admin/platform-bots/native/transports", {
          authorityGuard: guard(user?.id),
        }),
      ),
  });
}
export function useNativeEntryConversion() {
  const actor = useAuthStore((s) => s.user?.id);
  const client = useQueryClient();
  return useMutation({
    mutationFn: async (input: {
      channel_bot_id: string;
      funding_owner_user_id: string;
      transport: NativeTransport;
      recovery?: NativeSetupRecovery;
      lifecycle_review?: NativeLifecycleReview;
      replacement_connection_id?: string;
      public_key?: string;
    }) =>
      nativeConversionResponseSchema.parse(
        await apiClient("/admin/platform-bots/native", {
          method: "POST",
          body: input,
          authorityGuard: guard(actor),
        }),
      ),
    onSettled: () => {
      // A failed provider check can still leave a durable paused reservation.
      for (const queryKey of [
        ["admin-platform-profiles"],
        ["platform-runtime"],
        ["admin-platform-targets"],
      ])
        void client.invalidateQueries({ queryKey });
    },
  });
}
export function useNativeGatewayInspection() {
  const actor = useAuthStore((s) => s.user?.id);
  return useMutation({
    mutationFn: async (bot: string) =>
      nativeSetupRecoverySchema.parse(
        await apiClient(`/admin/platform-bots/native/${bot}/inspect`, {
          method: "POST",
          authorityGuard: guard(actor),
        }),
      ),
  });
}

export function useNativeLifecycleReview() {
  const actor = useAuthStore((s) => s.user?.id);
  return useMutation({
    mutationFn: async (bot: string) =>
      nativeLifecycleReviewSchema.parse(
        await apiClient(`/admin/platform-bots/native/${bot}/review`, {
          method: "POST",
          authorityGuard: guard(actor),
        }),
      ),
  });
}
