import { useInfiniteQuery } from "@tanstack/react-query";
import { assistantJson } from "@/lib/assistant/assistant-http";
import { ApiError } from "@/lib/api-error";
import { orgGuestHistorySchema, orgGuestIndexSchema } from "@/schemas/org-guest-conversation";
import { useAuthStore } from "@/stores/auth-store";

const root = "/assistant/nyxagent";

export function useOrgGuestConversations(agentId?: string) {
  const userId = useAuthStore((state) => state.user?.id);
  return useInfiniteQuery({
    queryKey: ["org-guest-conversations", userId, agentId],
    enabled: Boolean(userId && agentId),
    initialPageParam: null as string | null,
    queryFn: async ({ pageParam, signal }) => {
      const query = new URLSearchParams({ limit: "50" });
      if (pageParam) query.set("cursor", pageParam);
      try {
        return orgGuestIndexSchema.parse(await assistantJson(
          `${root}/agents/${encodeURIComponent(agentId!)}/guest-conversations?${query}`,
          { signal, reportErrors: false },
        ));
      } catch (error) {
        if (error instanceof ApiError && error.status === 404 && pageParam === null) {
          return { conversations: [], next_cursor: null };
        }
        throw error;
      }
    },
    getNextPageParam: (page) => page.next_cursor ?? undefined,
    gcTime: 0,
    retry: false,
  });
}

export function useOrgGuestHistory(id: string) {
  const userId = useAuthStore((state) => state.user?.id);
  return useInfiniteQuery({
    queryKey: ["org-guest-history", userId, id],
    enabled: Boolean(userId),
    initialPageParam: null as number | null,
    queryFn: async ({ pageParam, signal }) => {
      const query = new URLSearchParams({ limit: "50" });
      if (pageParam !== null) query.set("before_seq", String(pageParam));
      const page = orgGuestHistorySchema.parse(await assistantJson(
        `${root}/guest-conversations/${encodeURIComponent(id)}?${query}`,
        { signal },
      ));
      if (page.conversation.id !== id) throw new Error("Guest conversation mismatch");
      return page;
    },
    getNextPageParam: (page) => page.before_seq ?? undefined,
    gcTime: 0,
    retry: false,
  });
}
