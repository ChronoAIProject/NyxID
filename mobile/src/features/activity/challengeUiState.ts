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

const DEFAULT_GRANT_EXPIRY_DAYS = 30;

export function formatGrantDuration(grantExpiryDays?: number): string {
  const normalizedDays =
    typeof grantExpiryDays === "number" && Number.isFinite(grantExpiryDays) && grantExpiryDays > 0
      ? Math.floor(grantExpiryDays)
      : DEFAULT_GRANT_EXPIRY_DAYS;
  const label = normalizedDays === 1 ? "day" : "days";
  return `${normalizedDays} ${label}`;
}

export function getChallengeActionState(
  challenge: Pick<ChallengeDetail, "status" | "expires_at">
): ChallengeActionState {
  // Check definitive statuses first — these take precedence over time-based expiry
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

  if (challenge.status === "EXPIRED") {
    return {
      statusLabel: "EXPIRED",
      canDecide: false,
      reason: "This challenge has expired. Decision actions are disabled.",
    };
  }

  // Only apply time-based expiry to PENDING items
  const expiredByTime = Number.isFinite(Date.parse(challenge.expires_at))
    ? Date.parse(challenge.expires_at) <= Date.now()
    : false;

  if (expiredByTime) {
    return {
      statusLabel: "EXPIRED",
      canDecide: false,
      reason: "This challenge has expired. Decision actions are disabled.",
    };
  }

  return {
    statusLabel: "PENDING",
    canDecide: true,
    reason: null,
  };
}
