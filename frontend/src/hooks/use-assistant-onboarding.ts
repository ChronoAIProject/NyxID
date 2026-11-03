import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { create } from "zustand";
import { nyxBotApi } from "@/lib/assistant/nyxbot-api";
import { nyxBotQueryKeys } from "@/hooks/use-nyxbot-agents";
import type { AssistantOnboarding, OnboardingAnswers } from "@/schemas/assistant-onboarding";
import { useAuthStore } from "@/stores/auth-store";

/** First-run intro state, answer choices and next-step cards for the owner. */
export function useAssistantOnboarding(enabled = true) {
  const userId = useAuthStore((state) => state.user?.id);
  return useQuery({
    queryKey: nyxBotQueryKeys.onboarding(userId),
    queryFn: () => nyxBotApi.onboarding(),
    enabled: enabled && Boolean(userId),
    retry: false,
    staleTime: 60_000,
  });
}

export function useSaveOnboarding() {
  const userId = useAuthStore((state) => state.user?.id);
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (answers: OnboardingAnswers) => nyxBotApi.saveOnboarding(answers),
    onSuccess: (state) => {
      queryClient.setQueryData(nyxBotQueryKeys.onboarding(userId), state);
      // The answers become a NyxBot memory note.
      void queryClient.invalidateQueries({ queryKey: nyxBotQueryKeys.agents(userId) });
    },
  });
}

export function useDismissNextStep() {
  const userId = useAuthStore((state) => state.user?.id);
  const queryClient = useQueryClient();
  const key = nyxBotQueryKeys.onboarding(userId);
  return useMutation({
    mutationFn: (id: string) => nyxBotApi.dismissNextStep(id),
    onMutate: (id) => {
      queryClient.setQueryData<AssistantOnboarding>(key, (state) =>
        state
          ? { ...state, recommendations: state.recommendations.filter((step) => step.id !== id) }
          : state,
      );
    },
    onSuccess: (state) => queryClient.setQueryData(key, state),
    onError: () => queryClient.invalidateQueries({ queryKey: key }),
  });
}

/** Opens the intro again from the account menu. */
export const useIntroReplay = create<{
  readonly requested: number;
  readonly replay: () => void;
}>((set) => ({
  requested: 0,
  replay: () => set((state) => ({ requested: state.requested + 1 })),
}));
