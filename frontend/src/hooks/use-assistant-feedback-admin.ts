import { useInfiniteQuery, useQuery } from "@tanstack/react-query";
import { api } from "@/lib/api-client";
import {
  assistantFeedbackAdminSchema,
  feedbackThreadSchema,
} from "@/schemas/assistant-feedback-admin";
import { useAuthStore } from "@/stores/auth-store";

export function useAssistantFeedbackAdmin(
  params: URLSearchParams,
  enabled = true,
) {
  const userId = useAuthStore((state) => state.user?.id);
  return useQuery({
    queryKey: ["admin", "assistant-feedback", userId, params.toString()],
    queryFn: async () =>
      assistantFeedbackAdminSchema.parse(
        await api.get(`/admin/assistant/feedback?${params}`),
      ),
    retry: false,
    enabled,
    // Keep list rows while paging/filtering, but never carry totals across
    // accounts or date windows while the replacement request is pending.
    placeholderData: (previous, query) => {
      if (!query || query.queryKey[2] !== userId) return undefined;
      const before = new URLSearchParams(String(query.queryKey[3]));
      return before.get("from") === params.get("from") &&
        before.get("to") === params.get("to")
        ? previous
        : undefined;
    },
  });
}

export function useFeedbackThread(
  id: string,
  revision: string,
  enabled: boolean,
) {
  const userId = useAuthStore((state) => state.user?.id);
  return useInfiniteQuery({
    queryKey: ["admin", "feedback-thread", userId, id, revision],
    initialPageParam: undefined as string | undefined,
    queryFn: async ({ pageParam }) =>
      feedbackThreadSchema.parse(
        await api.get(
          `/admin/assistant/feedback/${encodeURIComponent(id)}/thread${pageParam ? `?cursor=${encodeURIComponent(pageParam)}` : ""}`,
        ),
      ),
    getNextPageParam: (page) => page.next_cursor ?? undefined,
    enabled,
    retry: false,
    staleTime: 0,
    gcTime: 0,
  });
}
