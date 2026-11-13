/** Server-authored evidence only; provider tool result text cannot set this. */
export interface ActivityDecision {
  readonly kind: string;
  readonly id: string;
  readonly state: string;
}

export function activityDecisionPresentation(decision: ActivityDecision | null | undefined) {
  if (!decision) return undefined;
  switch (decision.state) {
    case "waiting": return { label: "Waiting for approval", status: "waiting" as const, detail: "This step is paused for a decision. It has not failed." };
    case "allowed": return { label: "Allowed — waiting for execution", status: "waiting" as const, detail: "Approval was given. Completion of this action has not been confirmed." };
    case "completed": return { label: "Completed after approval", status: "done" as const, detail: "The approved action completed successfully. You do not need to retry it." };
    case "denied": return { label: "Declined", status: "waiting" as const, detail: "The request was declined. This approval did not authorize execution." };
    case "expired": return { label: "Approval expired", status: "waiting" as const, detail: "The approval expired. It is no longer available for execution." };
    case "cancelled": return { label: "Cancelled", status: "waiting" as const, detail: "The approved work was cancelled. Completion has not been confirmed." };
    case "failed": return { label: "Failed after approval", status: "error" as const, detail: "Approval was given, but the approved action failed. Check its outcome before retrying." };
    default: return { label: "Approval outcome unavailable", status: "waiting" as const, detail: "Completion of this action could not be confirmed. Check its outcome before retrying." };
  }
}
