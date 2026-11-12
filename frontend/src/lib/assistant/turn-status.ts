import { translateUi } from "@/i18n/use-ui-translation";
export type TurnPhase =
  | "approval"
  | "tool"
  | "retrying"
  | "continuing"
  | "working"
  | "active";

export interface TurnStatus {
  readonly phase: TurnPhase;
  readonly label: string;
  readonly startedAt?: string;
  readonly interrupted?: boolean;
  readonly toolLabel?: string;
  readonly toolStartedAt?: string;
}

export function deriveTurnStatus(input: {
  readonly activeTurn: {
    readonly activities?: readonly {
      status: string;
      label: string;
      started_at: string;
    }[];
    readonly continuations?: number;
    readonly started_at?: string;
  } | null | undefined;
  readonly live: boolean;
  readonly pendingApproval: boolean;
  readonly retrying: boolean;
  readonly interrupted?: boolean;
}): TurnStatus | null {
  if (!input.activeTurn) return null;
  const interrupted = input.interrupted;
  const startedAt = input.activeTurn.started_at;
  if (input.pendingApproval) {
    return { phase: "approval", get label() { return translateUi("Waiting for approval"); }, interrupted, startedAt };
  }
  const running = input.activeTurn.activities?.find(
    (activity) => activity.status === "running",
  );
  if (running) {
    return {
      phase: "tool",
      label: translateUi("Running {{label}}", { label: running.label }),
      toolLabel: running.label,
      toolStartedAt: running.started_at,
      interrupted,
      startedAt,
    };
  }
  if (input.retrying) return { phase: "retrying", get label() { return translateUi("Retrying…"); }, interrupted, startedAt };
  if ((input.activeTurn.continuations ?? 0) > 0) {
    return { phase: "continuing", get label() { return translateUi("Continuing task…"); }, interrupted, startedAt };
  }
  if (input.live) return { phase: "working", get label() { return translateUi("Working…"); }, interrupted, startedAt };
  return {
    phase: "active",
    get label() { return translateUi("Working — the reply appears when it is ready"); },
    interrupted,
    startedAt,
  };
}
