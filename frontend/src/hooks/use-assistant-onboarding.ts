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

function useSuggestionsVisibility(hidden: boolean) {
  const userId = useAuthStore((state) => state.user?.id);
  const queryClient = useQueryClient();
  const key = nyxBotQueryKeys.onboarding(userId);
  return useMutation({
    scope: { id: `assistant-onboarding-suggestions:${userId}` },
    mutationFn: () => hidden ? nyxBotApi.hideSuggestions() : nyxBotApi.showSuggestions(),
    onMutate: async () => {
      await queryClient.cancelQueries({ queryKey: key });
      const previous = queryClient.getQueryData<AssistantOnboarding>(key);
      queryClient.setQueryData<AssistantOnboarding>(key, (state) =>
        state ? { ...state, suggestions_hidden: hidden } : state,
      );
      return { previous };
    },
    onSuccess: (state) => queryClient.setQueryData(key, state),
    onError: (_error, _variables, context) => {
      if (context?.previous) queryClient.setQueryData(key, context.previous);
      return queryClient.invalidateQueries({ queryKey: key });
    },
  });
}

/** Hides the home's suggestions until restored or the intro is retaken. */
export function useHideSuggestions() {
  return useSuggestionsVisibility(true);
}

/** Restores suggestions without changing the intro answers or memory. */
export function useShowSuggestions() {
  return useSuggestionsVisibility(false);
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
