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
} from "../../lib/avatars/agentAvatars";
import type { AssistantConversation, AssistantMessage } from "./assistantTypes";

type ActiveTurn = NonNullable<AssistantConversation["active_turn"]>;
export type AgentTurnInput = Omit<AvatarMoodInput, "latestReply" | "celebrating">;

/**
 * Normalize the state the chat screen already holds; no new requests. The
 * web does the same in `frontend/src/lib/assistant/use-agent-mood.ts`.
 */
export function moodInputFor({
  activeTurn,
  sending,
  liveText,
  pendingOwnerAction,
}: {
  readonly activeTurn: ActiveTurn | null | undefined;
  /** This screen is running the turn (before the server row catches up). */
  readonly sending: boolean;
  /** Reply text streamed so far. */
  readonly liveText?: string;
  readonly pendingOwnerAction: boolean;
}): AgentTurnInput {
  const live = Boolean(activeTurn) || sending;
  return {
    activeTurn: live
      ? {
          stopRequested: activeTurn?.stop_requested,
          queued: activeTurn?.queued,
          waitingForCapacity: activeTurn?.waiting_for_capacity,
          continuations: activeTurn?.continuations,
          activityStatuses: activeTurn?.activities.map((activity) => activity.status),
          replyStreaming: Boolean(activeTurn?.reply?.text || liveText),
        }
      : null,
    pendingOwnerAction,
  };
}

/**
 * The latest settled assistant reply in a transcript. A stopped turn is
 * stored as a failure with `error_code: "cancelled"`; it settles to idle,
 * never to oops.
 */
export function settledReplyOf(
  messages: readonly AssistantMessage[] | undefined,
): AvatarSettledReply | null {
  if (!messages) return null;
  for (let index = messages.length - 1; index >= 0; index -= 1) {
    const message = messages[index]!;
    if (message.role !== "assistant") continue;
    if (message.status === "completed") return { id: message.id, outcome: "completed" };
    if (message.status === "failed") {
      return {
        id: message.id,
        outcome: message.error_code === "cancelled" ? "stopped" : "failed",
      };
    }
  }
  return null;
}

/**
 * The live mood for one conversation. A turn that ends while this screen
 * watches is judged by the reply it settles to: `happy` briefly for a
 * completion, `oops` for a failure, `idle` for a stop; until that reply
 * arrives the avatar stays neutral. Reopening a finished thread stays idle.
 * Decorative: the header keeps its status text.
 */
export function useAgentMood(
  conversationId: string | undefined,
  input: AgentTurnInput,
  reply: AvatarSettledReply | null,
): AvatarMood {
  const observed = { conversationId, active: Boolean(input.activeTurn), reply };
  const [settlement, setSettlement] = useState(() =>
    settleAvatarMood(undefined, observed),
  );
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
 * A working group member's mood from what the group screen holds. Groups
 * expose only which members are working (`working_agent_ids`) and the
 * members' pending cards; member turns' activities, queue state and streamed
 * text are not on the group transcript, so a working member is `thinking`,
 * or `asking` while one of its cards waits for a decision.
 */
export function groupMemberMood(
  memberId: string,
  pendingActions: readonly { readonly agent_id?: string | null }[],
): AvatarMood {
  return avatarMood({
    activeTurn: {},
    pendingOwnerAction: pendingActions.some((action) => action.agent_id === memberId),
  });
}
