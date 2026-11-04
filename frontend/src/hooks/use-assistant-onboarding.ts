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

/** Hides the home's suggestions until the intro is retaken. */
export function useHideSuggestions() {
  const userId = useAuthStore((state) => state.user?.id);
  const queryClient = useQueryClient();
  const key = nyxBotQueryKeys.onboarding(userId);
  return useMutation({
    mutationFn: () => nyxBotApi.hideSuggestions(),
    onMutate: () => {
      queryClient.setQueryData<AssistantOnboarding>(key, (state) =>
        state ? { ...state, suggestions_hidden: true } : state,
      );
    },
    onSuccess: (state) => queryClient.setQueryData(key, state),
    onError: () => queryClient.invalidateQueries({ queryKey: key }),
  });
}

/**
 * A request to open the intro again, from the account menu on any assistant
 * page. The chat page consumes it once it is showing.
 */
export const useIntroReplay = create<{
  readonly pending: boolean;
  readonly replay: () => void;
  readonly consume: () => void;
}>((set) => ({
  pending: false,
  replay: () => set({ pending: true }),
  consume: () => set({ pending: false }),
}));
