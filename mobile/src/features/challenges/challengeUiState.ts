import { ChallengeDetail } from "../../lib/api/types";

export type ChallengeActionState = {
  statusLabel: string;
  canDecide: boolean;
  reason: string | null;
};

export function getErrorCode(error: unknown): string | null {
  if (error instanceof Error && typeof error.message === "string") {
    return error.message;
  }
  return null;
}

export function getChallengeQueryErrorMessage(error: unknown): string {
  const code = getErrorCode(error);
  if (code === "challenge_not_found") {
    return "This challenge does not exist or has been removed.";
  }
  return "Failed to load data. Please try again.";
}

export function getDecisionErrorMessage(error: unknown): string {
  const code = getErrorCode(error);
  if (code === "already_decided") {
    return "This challenge has already been processed.";
  }
  if (code === "challenge_not_found") {
    return "This challenge does not exist or is no longer valid.";
  }
  return "Action failed. Please retry.";
}

export function getChallengeActionState(
  challenge: Pick<ChallengeDetail, "status" | "expires_at">
): ChallengeActionState {
  const expiredByTime = Number.isFinite(Date.parse(challenge.expires_at))
    ? Date.parse(challenge.expires_at) <= Date.now()
    : false;

  if (challenge.status === "EXPIRED" || expiredByTime) {
    return {
      statusLabel: "EXPIRED",
      canDecide: false,
      reason: "This challenge has expired. Decision actions are disabled.",
    };
  }

  if (challenge.status === "APPROVED") {
    return {
      statusLabel: "APPROVED",
      canDecide: false,
      reason: "This challenge has already been approved.",
    };
  }

  if (challenge.status === "DENIED") {
    return {
      statusLabel: "DENIED",
      canDecide: false,
      reason: "This challenge has already been denied.",
    };
  }

  return {
    statusLabel: "PENDING",
    canDecide: true,
    reason: null,
  };
}
