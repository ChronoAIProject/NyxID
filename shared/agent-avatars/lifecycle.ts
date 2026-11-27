import type { AvatarMood, ProcessState } from "./model";
import { resolveTurnActivity } from "./activity";

/** How a settled reply ended. Streaming replies are not settled. */
export type AvatarReplyOutcome = "completed" | "failed" | "stopped";

/** The facts both clients already hold, normalized. Mood is decorative: the
 * existing status UI keeps carrying the state. */
export interface AvatarMoodInput {
  readonly activeTurn?: {
    readonly stopRequested?: boolean;
    readonly queued?: boolean;
    readonly waitingForCapacity?: boolean;
    readonly continuations?: number;
    /** `running`, `completed` or `error` per activity in this turn. */
    readonly activityStatuses?: readonly string[];
    /** Reply text has started streaming. */
    readonly replyStreaming?: boolean;
  } | null;
  /** An owner action card / acknowledgement waits in this conversation. */
  readonly pendingOwnerAction?: boolean;
  /** How the latest settled reply ended; null while unknown. */
  readonly latestReply?: AvatarReplyOutcome | null;
  /** A successful completion was observed live (see `settleAvatarMood`). */
  readonly celebrating?: boolean;
}

/** How long the one-shot `happy` mood shows after a live completion. */
export const AVATAR_HAPPY_MS = 1600;

export function avatarMood(input: AvatarMoodInput): ProcessState {
  const turn = input.activeTurn;
  const state = resolveTurnActivity(
    turn
      ? {
          stop_requested: turn.stopRequested,
          queued: turn.queued,
          waiting_for_capacity: turn.waitingForCapacity,
          continuations: turn.continuations,
          activities: turn.activityStatuses?.map((status) => ({ status })),
        }
      : null,
    {
      pendingOwnerAction: input.pendingOwnerAction,
      streamedTextPresent: turn?.replyStreaming,
    },
  );
  if (state !== "idle") return state;
  if (input.latestReply === "failed") return "failure";
  if (input.latestReply === "completed" && input.celebrating) return "success";
  return "idle";
}

/** The latest settled reply a page shows, by id. */
export interface AvatarSettledReply {
  readonly id: string;
  readonly outcome: AvatarReplyOutcome;
}

/**
 * Live settlement tracking for one view. A turn that ends while watched is
 * judged by the reply it settles to, never by that reply's absence: until a
 * newer settled reply arrives the avatar stays neutral, so a late failure
 * never flashes `happy` first. Returns `previous` unchanged when nothing
 * moved, so callers can store it in state.
 */
export interface AvatarSettlement {
  readonly conversationId: string | undefined;
  readonly active: boolean;
  /** The settled reply that was latest when the last turn began. */
  readonly baselineReplyId: string | null;
  /** A watched turn ended and its settled reply has not arrived yet. */
  readonly awaiting: boolean;
  /** The completed reply being celebrated, until `endCelebration`. */
  readonly celebrateReplyId: string | null;
}

export function settleAvatarMood(
  previous: AvatarSettlement | undefined,
  observed: {
    readonly conversationId: string | undefined;
    readonly active: boolean;
    readonly reply: AvatarSettledReply | null;
  },
): AvatarSettlement {
  const replyId = observed.reply?.id ?? null;
  if (!previous || previous.conversationId !== observed.conversationId) {
    // A thread opened (or reopened) settled never celebrates.
    return {
      conversationId: observed.conversationId,
      active: observed.active,
      baselineReplyId: replyId,
      awaiting: false,
      celebrateReplyId: null,
    };
  }
  if (!previous.active && observed.active) {
    return {
      ...previous,
      active: true,
      baselineReplyId: previous.awaiting ? previous.baselineReplyId : replyId,
      awaiting: false,
      celebrateReplyId: null,
    };
  }
  if (previous.active && !observed.active) {
    const settled =
      observed.reply && replyId !== previous.baselineReplyId
        ? observed.reply
        : null;
    return {
      ...previous,
      active: false,
      awaiting: settled === null,
      baselineReplyId: settled ? settled.id : previous.baselineReplyId,
      celebrateReplyId: settled?.outcome === "completed" ? settled.id : null,
    };
  }
  if (
    previous.awaiting &&
    observed.reply &&
    replyId !== previous.baselineReplyId
  ) {
    return {
      ...previous,
      awaiting: false,
      baselineReplyId: replyId,
      celebrateReplyId:
        observed.reply.outcome === "completed" ? observed.reply.id : null,
    };
  }
  return previous;
}

/** The celebration's one-shot window ended. */
export function endCelebration(settlement: AvatarSettlement): AvatarSettlement {
  return settlement.celebrateReplyId === null
    ? settlement
    : { ...settlement, celebrateReplyId: null };
}

/** The mood for a view, given its tracked settlement and current reply. */
export function settledAvatarMood(
  input: Omit<AvatarMoodInput, "latestReply" | "celebrating">,
  settlement: AvatarSettlement,
  reply: AvatarSettledReply | null,
): AvatarMood {
  return avatarMood({
    ...input,
    latestReply: settlement.awaiting ? null : (reply?.outcome ?? null),
    celebrating:
      settlement.celebrateReplyId !== null &&
      reply?.id === settlement.celebrateReplyId,
  });
}
