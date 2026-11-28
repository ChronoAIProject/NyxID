import { translateApp } from "../../i18n/appTranslation";
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
    return translateApp("This challenge does not exist or has been removed.");
  }
  return translateApp("Failed to load data. Please try again.");
}

export function getDecisionErrorMessage(error: unknown): string {
  const code = getErrorCode(error);
  if (code === "already_decided") {
    return translateApp("This challenge has already been processed.");
  }
  if (code === "challenge_not_found") {
    return translateApp("This challenge does not exist or is no longer valid.");
  }
  return translateApp("Action failed. Please retry.");
}

const DEFAULT_GRANT_EXPIRY_DAYS = 30;

export function formatGrantDuration(grantExpiryDays?: number): string {
  const normalizedDays =
    typeof grantExpiryDays === "number" &&
    Number.isFinite(grantExpiryDays) &&
    grantExpiryDays > 0
      ? Math.floor(grantExpiryDays)
      : DEFAULT_GRANT_EXPIRY_DAYS;
  return translateApp(normalizedDays === 1 ? "{{days}} day" : "{{days}} days", { days: normalizedDays });
}

export function getChallengeActionState(
  challenge: Pick<ChallengeDetail, "status" | "expires_at" | "execution_proposal" | "decision_destination">,
): ChallengeActionState {
  if (challenge.execution_proposal && challenge.decision_destination !== "originating_owner_chat") {
    return {
      statusLabel: challenge.execution_proposal.state,
      canDecide: false,
      reason: translateApp("Review this execution permission in the web app."),
    };
  }
  // Check definitive statuses first — these take precedence over time-based expiry
  if (challenge.status === "APPROVED") {
    return {
      statusLabel: translateApp("Approved"),
      canDecide: false,
      reason: translateApp("This challenge has already been approved."),
    };
  }

  if (challenge.status === "DENIED") {
    return {
      statusLabel: translateApp("Denied"),
      canDecide: false,
      reason: translateApp("This challenge has already been denied."),
    };
  }

  if (challenge.status === "EXPIRED") {
    return {
      statusLabel: translateApp("Expired"),
      canDecide: false,
      reason: translateApp(
        "This challenge has expired. Decision actions are disabled.",
      ),
    };
  }

  if (!Number.isFinite(Date.parse(challenge.expires_at))) {
    return {
      statusLabel: translateApp("Pending"),
      canDecide: false,
      reason: translateApp(
        "The request expiry could not be verified. Refresh before deciding.",
      ),
    };
  }

  const expiredByTime = Date.parse(challenge.expires_at) <= Date.now();

  if (expiredByTime) {
    return {
      statusLabel: translateApp("Expired"),
      canDecide: false,
      reason: translateApp(
        "This challenge has expired. Decision actions are disabled.",
      ),
    };
  }

  if (challenge.decision_destination === "originating_owner_chat") {
    return {
      statusLabel: translateApp("Pending"),
      canDecide: false,
      reason: translateApp("Review this request in your original private chat with NyxBot."),
    };
  }
  return {
    statusLabel: translateApp("Pending"),
    canDecide: true,
    reason: null,
  };
}
