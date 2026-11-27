import { useEffect, useState } from "react";
import {
  AVATAR_HAPPY_MS,
  avatarMood,
  endCelebration,
  settleAvatarMood,
  settledAvatarMood,
  type AvatarMood,
  type AvatarMoodInput,
  type AvatarSettledReply,
} from "@/lib/assistant/agent-avatar-model";
import type { ChatMessage } from "@/lib/assistant/chat-types";
import type { AssistantGroupPendingAction, NyxAgentConversation } from "@/schemas/assistant-nyxagent";

type ActiveTurn = NonNullable<NyxAgentConversation["active_turn"]>;
type TurnInput = Omit<AvatarMoodInput, "latestReply" | "celebrating">;

/** Normalize the state the chat page already holds; no new fetches. */
export function moodInputFor({
  activeTurn,
  streaming,
  streamedText,
  pendingOwnerAction,
}: {
  readonly activeTurn: ActiveTurn | null | undefined;
  /** This page is running the turn (before the server row catches up). */
  readonly streaming: boolean;
  /** Reply text this page has streamed so far. */
  readonly streamedText?: string;
  readonly pendingOwnerAction: boolean;
}): TurnInput {
  const live = Boolean(activeTurn) || streaming;
  return {
    activeTurn: live
      ? {
          stopRequested: activeTurn?.stop_requested,
          queued: activeTurn?.queued,
          waitingForCapacity: activeTurn?.waiting_for_capacity,
          continuations: activeTurn?.continuations,
          activityStatuses: activeTurn?.activities.map((activity) => activity.status),
          replyStreaming: Boolean(activeTurn?.reply?.text || streamedText),
        }
      : null,
    pendingOwnerAction,
  };
}

/** The latest settled assistant reply in a transcript (streaming replies are
 * not settled yet). */
export function settledReplyOf(messages: readonly ChatMessage[]): AvatarSettledReply | null {
  for (let index = messages.length - 1; index >= 0; index -= 1) {
    const message = messages[index]!;
    if (message.role !== "assistant") continue;
    if (message.status === "complete") return { id: message.id, outcome: "completed" };
    if (message.status === "error") return { id: message.id, outcome: "failed" };
    if (message.status === "stopped") return { id: message.id, outcome: "stopped" };
  }
  return null;
}

/**
 * The live mood for one conversation. A turn that ends while this page
 * watches is judged by the reply it settles to: `happy` briefly for a
 * completion, `oops` for a failure, `idle` for a stop; until that reply
 * arrives the avatar stays neutral. Reopening a finished thread stays idle.
 * Decorative: callers keep their status UI.
 */
export function useAgentMood(
  conversationId: string | undefined,
  input: TurnInput,
  reply: AvatarSettledReply | null,
): AvatarMood {
  const observed = { conversationId, active: Boolean(input.activeTurn), reply };
  const [settlement, setSettlement] = useState(() => settleAvatarMood(undefined, observed));
  const next = settleAvatarMood(settlement, observed);
  // Adjust during render when the thread, turn or settled reply changes.
  if (next !== settlement) setSettlement(next);
  const celebrating = next.celebrateReplyId;
  useEffect(() => {
    if (celebrating === null) return;
    const timer = setTimeout(() => setSettlement(endCelebration), AVATAR_HAPPY_MS);
    return () => clearTimeout(timer);
  }, [celebrating]);
  return settledAvatarMood(input, next, reply);
}

/**
 * A working member's mood from what the group page holds. Groups expose only
 * which members are working (`working_agent_ids`) and the members' pending
 * cards; member turns' activities, queue state and streamed text are not on
 * the group transcript, so a working member is `thinking`, or `asking` while
 * one of its cards waits for a decision.
 */
export function groupMemberMood(
  memberId: string,
  pendingActions: readonly Pick<AssistantGroupPendingAction, "agent_id">[],
): AvatarMood {
  return avatarMood({
    activeTurn: {},
    pendingOwnerAction: pendingActions.some((action) => action.agent_id === memberId),
  });
}
