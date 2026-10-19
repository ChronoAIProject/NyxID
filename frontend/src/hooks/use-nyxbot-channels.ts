import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { ApiError } from "@/lib/api-client";
import {
  getNyxbotRegistrationStatus,
  registerNyxbotTelegram,
} from "@/lib/nyxbot-channels";
import { channelBotsQueryKeys } from "@/hooks/use-channel-bots";
import { useAuthStore } from "@/stores/auth-store";

export function useRegisterNyxbotTelegram() {
  const queryClient = useQueryClient();
  return useMutation({
    gcTime: 0,
    retry: false,
    mutationFn: ({
      botToken,
      serviceIds,
    }: {
      botToken: string;
      serviceIds: readonly string[];
    }) => registerNyxbotTelegram(botToken, serviceIds),
    onSuccess: () => {
      void queryClient.invalidateQueries({
        queryKey: channelBotsQueryKeys.all,
      });
    },
  });
}

export function useNyxbotRegistrationStatus(registrationId: string | null) {
  const userId = useAuthStore((s) => s.user?.id);
  return useQuery({
    queryKey: ["nyxbot-registration", userId, registrationId],
    queryFn: () => getNyxbotRegistrationStatus(registrationId!),
    enabled: Boolean(userId && registrationId),
    // Aevatar accepts the mirror command before its status becomes readable.
    retry: (count, error) =>
      error instanceof ApiError && error.status === 404 && count < 3,
    retryDelay: 2_000,
  });
}
