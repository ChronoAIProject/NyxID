import type { ProcessState } from "./model";

/** Precedence across conversations for the same agent. */
export const ROSTER_ACTIVITY_STATES = [
  "approval",
  "working",
  "drafting",
  "retry",
  "thinking",
  "waiting",
  "stopping",
] as const satisfies readonly ProcessState[];

export type RosterActivityState = (typeof ROSTER_ACTIVITY_STATES)[number];
export type AgentRosterMood = RosterActivityState | "idle";

export const ROSTER_ACTIVITY_LABELS = {
  approval: "Waiting for approval",
  working: "Working",
  drafting: "Writing",
  retry: "Retrying",
  thinking: "Thinking",
  waiting: "Waiting",
  stopping: "Stopping",
} as const satisfies Record<RosterActivityState, string>;

export interface TurnActivityFacts {
  readonly stop_requested?: boolean;
  readonly queued?: boolean;
  readonly waiting_for_capacity?: boolean;
  readonly continuations?: number;
  readonly activities?: readonly { readonly status: string }[];
  readonly reply?: { readonly text?: string | null } | null;
}

/** Facts from a turn already known to be live, or no turn. Pending owner
 * actions belong to the conversation and can outlive an individual turn. */
export function resolveTurnActivity(
  turn: TurnActivityFacts | null | undefined,
  {
    pendingOwnerAction = false,
    streamedTextPresent = false,
  }: {
    readonly pendingOwnerAction?: boolean;
    readonly streamedTextPresent?: boolean;
  } = {},
): AgentRosterMood {
  if (turn?.stop_requested) return "stopping";
  if (pendingOwnerAction) return "approval";
  if (!turn) return "idle";
  if (turn.queued || turn.waiting_for_capacity) return "waiting";
  if (turn.activities?.some(({ status }) => status === "running"))
    return "working";
  if (turn.reply?.text || streamedTextPresent) return "drafting";
  if (
    (turn.continuations ?? 0) > 0 ||
    (turn.activities?.filter(({ status }) => status === "error").length ?? 0) >=
      2
  ) {
    return "retry";
  }
  return "thinking";
}

export interface AgentThreadActivity {
  readonly conversation_id: string;
  readonly state: RosterActivityState;
}

export interface AgentRosterMoodInput {
  readonly agentId: string;
  readonly activity?: readonly AgentThreadActivity[];
  readonly pendingOwnerActions?: number;
  readonly running?: boolean;
  readonly destroyed?: boolean;
  readonly override?: {
    readonly agentId: string;
    readonly conversationId: string;
    readonly state: ProcessState;
  };
}

export function agentRosterMood({
  agentId,
  activity,
  pendingOwnerActions = 0,
  running,
  destroyed,
  override,
}: AgentRosterMoodInput): AgentRosterMood {
  if (destroyed) return "idle";
  if (!activity && pendingOwnerActions > 0) return "approval";
  const watched = override?.agentId === agentId ? override : undefined;

  // A watched thread replaces its older roster observation, including at
  // settlement. Only live roster states can win the rank below.
  const states = new Set<ProcessState>(
    activity
      ?.filter(
        ({ conversation_id }) => conversation_id !== watched?.conversationId,
      )
      .map(({ state }) => state),
  );
  // An older server's running flag can refer to a different thread. Keep
  // that candidate until the roster refreshes after settlement (2 s flush).
  if (!activity && running) states.add("thinking");
  if (watched) states.add(watched.state);
  return ROSTER_ACTIVITY_STATES.find((state) => states.has(state)) ?? "idle";
}
