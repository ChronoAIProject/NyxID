import { useCallback, useState } from "react";
import { api } from "@/lib/api-client";

export type AssistantFeedbackVote = "up" | "down";
export interface AssistantFeedback { readonly vote: AssistantFeedbackVote; readonly comment?: string | null; readonly updated_at?: string; }
export function useAssistantFeedback(scope: "conversation" | "group", parentId: string, messageId: string) {
  const path = `/assistant/nyxagent/${scope === "group" ? "groups" : "conversations"}/${encodeURIComponent(parentId)}/messages/${encodeURIComponent(messageId)}/feedback`;
  const [isPending, setPending] = useState(false);
  const mutateAsync = useCallback(async ({ vote, comment }: { vote?: AssistantFeedbackVote; comment?: string }) => {
    setPending(true);
    try { return vote ? await api.put<AssistantFeedback>(path, { vote, comment }) : await api.delete<void>(path); }
    finally { setPending(false); }
  }, [path]);
  return { mutateAsync, isPending };
}
