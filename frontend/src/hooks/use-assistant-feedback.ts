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

export interface ReplyFeedback {
  readonly vote: AssistantFeedbackVote | undefined;
  readonly pending: boolean;
  readonly formOpen: boolean;
  /** The vote itself failed to save. */
  readonly error: boolean;
  readonly toggle: (next: AssistantFeedbackVote) => Promise<void>;
  /** Saves the optional details and closes the form; false when saving failed. */
  readonly send: (comment: string) => Promise<boolean>;
  readonly close: () => void;
}

/**
 * One reply's vote. Choosing a vote saves it at once and opens the optional
 * reasons and details form; choosing it again withdraws it.
 */
export function useReplyFeedback({
  scope,
  parentId,
  messageId,
  initialVote,
  onVoted,
}: {
  readonly scope: "conversation" | "group";
  readonly parentId: string;
  readonly messageId: string;
  readonly initialVote: AssistantFeedbackVote | undefined;
  readonly onVoted?: (vote: AssistantFeedbackVote) => void;
}): ReplyFeedback {
  const feedback = useAssistantFeedback(scope, parentId, messageId);
  const [vote, setVote] = useState(initialVote);
  const [formOpen, setFormOpen] = useState(false);
  const [error, setError] = useState(false);
  const toggle = async (next: AssistantFeedbackVote) => {
    const previous = vote;
    const selected = previous === next ? undefined : next;
    setVote(selected);
    setFormOpen(Boolean(selected));
    try {
      await feedback.mutateAsync(selected ? { vote: selected } : {});
      setError(false);
    } catch {
      setVote(previous);
      setFormOpen(false);
      setError(true);
      return;
    }
    if (selected) onVoted?.(selected);
  };
  const send = async (comment: string) => {
    if (!vote) return false;
    // The vote is already saved; empty details leave it as it is.
    if (comment) {
      try {
        await feedback.mutateAsync({ vote, comment });
      } catch {
        return false;
      }
    }
    setFormOpen(false);
    return true;
  };
  return {
    vote,
    pending: feedback.isPending,
    formOpen,
    error,
    toggle,
    send,
    close: () => setFormOpen(false),
  };
}
