import { useCallback, useContext, useState } from "react";
import { api } from "@/lib/api-client";
import { QueryClientContext } from "@tanstack/react-query";
import { nyxBotQueryKeys } from "@/hooks/use-nyxbot-agents";
import { nyxBotGroupKeys } from "@/hooks/use-nyxbot-groups";
import { useAuthStore } from "@/stores/auth-store";

export type AssistantFeedbackVote = "up" | "down";
export interface AssistantFeedback {
  readonly vote: AssistantFeedbackVote;
  readonly comment?: string | null;
  readonly updated_at?: string;
}
export function useAssistantFeedback(
  scope: "conversation" | "group",
  parentId: string,
  messageId: string,
) {
  const path = `/assistant/nyxagent/${scope === "group" ? "groups" : "conversations"}/${encodeURIComponent(parentId)}/messages/${encodeURIComponent(messageId)}/feedback`;
  const [isPending, setPending] = useState(false);
  const queryClient = useContext(QueryClientContext);
  const userId = useAuthStore((state) => state.user?.id);
  const mutateAsync = useCallback(
    async ({
      vote,
      comment,
    }: {
      vote?: AssistantFeedbackVote;
      comment?: string;
    }) => {
      setPending(true);
      try {
        const result = vote
          ? await api.put<AssistantFeedback>(path, { vote, comment })
          : await api.delete<void>(path);
        if (queryClient) {
          const queryKey =
            scope === "conversation"
              ? nyxBotQueryKeys.history(userId, parentId)
              : nyxBotGroupKeys.messages(userId, parentId);
          await queryClient.invalidateQueries({ queryKey });
        }
        return result;
      } finally {
        setPending(false);
      }
    },
    [path, queryClient, scope, parentId, userId],
  );
  return {
    mutateAsync,
    isPending,
    cacheKey: [scope, parentId, messageId] as const,
  };
}
