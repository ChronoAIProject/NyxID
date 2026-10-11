import { useCallback, useContext, useState } from "react";
import { api } from "@/lib/api-client";
import { QueryClientContext } from "@tanstack/react-query";

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
        if (queryClient)
          await queryClient.invalidateQueries({
            queryKey: ["assistant", "nyxagent", scope, parentId],
          });
        return result;
      } finally {
        setPending(false);
      }
    },
    [path, queryClient, scope, parentId],
  );
  return {
    mutateAsync,
    isPending,
    cacheKey: [scope, parentId, messageId] as const,
  };
}
