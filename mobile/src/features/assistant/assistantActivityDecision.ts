import { translateApp } from "../../i18n/appTranslation";
/** Server-authored evidence only; provider tool result text cannot set this. */
export interface ActivityDecision {
  readonly kind: string;
  readonly id: string;
  readonly state: string;
}

export function activityDecisionPresentation(decision: ActivityDecision | null | undefined) {
  if (!decision) return undefined;
  switch (decision.state) {
    case "waiting": return { label: translateApp("Waiting for approval"), status: "waiting" as const, detail: translateApp(
          "This step is paused for a decision. It has not failed.",
        ),
      };
    case "allowed": return { label: translateApp("Allowed — waiting for execution"), status: "waiting" as const, detail: translateApp(
          "Approval was given. Completion of this action has not been confirmed.",
        ),
      };
    case "completed": return { label: translateApp("Completed after approval"), status: "done" as const, detail: translateApp(
          "The approved action completed successfully. You do not need to retry it.",
        ),
      };
    case "denied": return { label: translateApp("Declined"), status: "waiting" as const, detail: translateApp(
          "The request was declined. This approval did not authorize execution.",
        ),
      };
    case "expired": return { label: translateApp("Approval expired"), status: "waiting" as const, detail: translateApp(
          "The approval expired. It is no longer available for execution.",
        ),
      };
    case "cancelled": return { label: translateApp("Cancelled"), status: "waiting" as const, detail: translateApp(
          "The approved work was cancelled. Completion has not been confirmed.",
        ),
      };
    case "failed": return { label: translateApp("Failed after approval"), status: "error" as const, detail: translateApp(
          "Approval was given, but the approved action failed. Check its outcome before retrying.",
        ),
      };
    default: return { label: translateApp("Approval outcome unavailable"), status: "waiting" as const, detail: translateApp(
          "Completion of this action could not be confirmed. Check its outcome before retrying.",
        ),
      };
  }
}
