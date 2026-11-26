import { useEffect, useState } from "react";
import { useQueries } from "@tanstack/react-query";
import { pendingChatDecisions } from "@/lib/assistant/pending-decisions";
import { authoredSkillPreviewQuery } from "@/lib/assistant/authored-skill-preview";
import { useAuthStore } from "@/stores/auth-store";
import type { NyxAgentAccessMode, NyxAgentAcknowledgement, NyxAgentApproval } from "@/schemas/assistant-nyxagent";
import { useSharedConsents } from "./use-shared-agents";

export function useChatDecisions({ acknowledgements, approvals, accessMode, sharedAgentId, conversationId, disabled }: {
  readonly acknowledgements: readonly NyxAgentAcknowledgement[];
  readonly approvals: readonly NyxAgentApproval[];
  readonly accessMode?: NyxAgentAccessMode;
  readonly sharedAgentId?: string;
  readonly conversationId?: string;
  readonly disabled?: boolean;
}) {
  const owner = useAuthStore((state) => state.user?.id);
  const consents = useSharedConsents(sharedAgentId ?? "", Boolean(sharedAgentId && conversationId && !disabled));
  const rows = consents.data?.consents.filter((row) => row.conversation_id === conversationId) ?? [];
  const [now, setNow] = useState(() => Date.now());
  const authored = acknowledgements.filter((card) => card.authored_skill && card.status === "pending");
  const previews = useQueries({ queries: authored.map((card) => ({
    ...authoredSkillPreviewQuery(owner, card), enabled: Boolean(owner && !disabled),
  })) });
  const expirations = [...acknowledgements, ...approvals, ...rows]
    .map((row) => Date.parse(row.expires_at ?? ""))
    .filter((expiry) => expiry > now);
  const nextExpiry = Math.min(...expirations);
  useEffect(() => {
    // Expire a card even when its history poll is idle or the tab was asleep.
    const refresh = () => setNow(Date.now());
    const timer = Number.isFinite(nextExpiry)
      ? setTimeout(refresh, Math.min(2_147_483_647, Math.max(1, nextExpiry - Date.now())))
      : undefined;
    window.addEventListener("focus", refresh);
    document.addEventListener("visibilitychange", refresh);
    return () => {
      clearTimeout(timer);
      window.removeEventListener("focus", refresh);
      document.removeEventListener("visibilitychange", refresh);
    };
  }, [nextExpiry, now]);
  return {
    decisions: disabled ? [] : pendingChatDecisions({
      acknowledgements, approvals, consents: rows, accessMode, now,
      authoredPreviews: new Map(authored.map((card, index) => [card.id, previews[index]!])),
    }),
    consentError: sharedAgentId ? consents.error : null,
  };
}
