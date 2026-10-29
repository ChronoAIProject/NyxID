import type { AssistantMessage } from "./assistantTypes";

// Behavior and copy follow the web NyxAgent turn presentation.
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
  readonly activeTurn:
    | {
        readonly activities?: readonly {
          status: string;
          label: string;
          started_at: string;
        }[];
        readonly continuations?: number;
        readonly started_at?: string;
      }
    | null
    | undefined;
  readonly live: boolean;
  readonly pendingApproval: boolean;
  readonly retrying: boolean;
  readonly interrupted?: boolean;
}): TurnStatus | null {
  if (!input.activeTurn) return null;
  const interrupted = input.interrupted;
  const startedAt = input.activeTurn.started_at;
  if (input.pendingApproval) {
    return {
      phase: "approval",
      label: "Waiting for approval",
      interrupted,
      startedAt,
    };
  }
  const running = input.activeTurn.activities?.find(
    (activity) => activity.status === "running",
  );
  if (running) {
    return {
      phase: "tool",
      label: `Running ${running.label}`,
      toolLabel: running.label,
      toolStartedAt: running.started_at,
      interrupted,
      startedAt,
    };
  }
  if (input.retrying)
    return { phase: "retrying", label: "Retrying…", interrupted, startedAt };
  if ((input.activeTurn.continuations ?? 0) > 0) {
    return {
      phase: "continuing",
      label: "Continuing task…",
      interrupted,
      startedAt,
    };
  }
  if (input.live)
    return { phase: "working", label: "Working…", interrupted, startedAt };
  return {
    phase: "active",
    label: "Working — the reply appears when it is ready",
    interrupted,
    startedAt,
  };
}

/** NyxID-authored pending note, mirrored from assistant_engine_policy.rs. */
export const CAPACITY_ACKNOWLEDGEMENT =
  "Got it — I'm a bit busy right now and will start on this shortly.";

/** Elapsed wait rounded up to a whole second, matching the server formatter. */
export function formatCapacityWait(ms: number): string {
  const total = Math.ceil(Math.max(0, ms) / 1000);
  const minutes = Math.floor(total / 60);
  const seconds = total % 60;
  const parts: string[] = [];
  if (minutes > 0) parts.push(`${minutes} minute${minutes === 1 ? "" : "s"}`);
  if (seconds > 0 || !parts.length)
    parts.push(`${seconds} second${seconds === 1 ? "" : "s"}`);
  return parts.join(" ");
}

interface BillingError {
  readonly title: string;
  readonly message: string;
  readonly openBilling: boolean;
}

export function billingError(
  code: string | null | undefined,
): BillingError | undefined {
  switch (code) {
    case "insufficient_credits":
      return {
        title: "Not enough credits",
        message:
          "There aren't enough credits to run this turn. Add credits in Billing & Usage, then try again.",
        openBilling: true,
      };
    case "wallet_suspended":
      return {
        title: "Billing wallet suspended",
        message:
          "Your billing wallet is suspended. Contact support or check Billing & Usage.",
        openBilling: true,
      };
    case "plan_entitlement_required":
      return {
        title: "Service not included in your plan",
        message:
          "Your plan does not include this service. Check your plan in Billing & Usage.",
        openBilling: true,
      };
    case "billing_not_configured":
      return {
        title: "Billing setup incomplete",
        message:
          "Billing setup is incomplete, so NyxBot could not run this turn. Contact support to complete the billing configuration.",
        openBilling: false,
      };
    case "billing_provider_unavailable":
      return {
        title: "Billing service unavailable",
        message:
          "NyxBot could not verify billing because the billing service is unavailable. Try again shortly; if this continues, contact support.",
        openBilling: false,
      };
    default:
      return undefined;
  }
}

/** Stable, safe copy shared by composer feedback and persisted transcript rows. */
export function steeringNotice(
  code: string | null | undefined,
  outcome?: string,
): string {
  if (outcome === "applied") return "Guidance applied to this reply.";
  switch (code) {
    case "starting":
      return "The assistant is starting. Try this guidance again shortly.";
    case "no_active_turn":
      return "This turn has ended. Send the guidance as a new message?";
    case "response_mismatch":
      return "The running response changed. Turn state refreshed; try your guidance again.";
    case "idempotency_conflict":
      return "This request ID was reused with different guidance. Reload before trying again.";
    case "steer_limit":
      return "This reply has reached its guidance limit. Wait for it to finish.";
    case "stop_pending":
      return "The assistant is stopping. Wait for it to finish.";
    case "steer_unsupported":
      return "Steering is not supported for this turn.";
    case "turn_active":
      return "Wait for this reply to finish before sending another message.";
    case "invalid_request":
      return "Guidance must be text only and at most 32,000 characters.";
    case "not_found":
      return "The assistant conversation was not found.";
    default:
      return "This guidance may not have been applied. It will not be sent again automatically.";
  }
}

export const CONTEXT_RESET_NOTICE =
  "The last turn failed, so the assistant continues in a fresh context with a recap of its completed actions, failures and attachments. Rerun only what did not complete.";

export function storedError(
  code: string | null,
  capacityWaitMs?: number | null,
): string {
  const billing = billingError(code);
  if (billing) return billing.message;
  if (code === "tool_budget_exhausted" || code === "turn_timeout")
    return "This task reached its automatic continuation limit. Its context is saved; resume to continue, or adjust the limit in NyxBot settings.";
  if (code === "continuation_no_progress")
    return "Paused because the task repeated without progress. Its context is saved; give it new guidance.";
  if (code === "idle_timeout")
    return "The assistant stopped responding. Check what it already did before asking it to continue.";
  if (code === "outcome_unknown") {
    return "The previous operation may have taken effect. Check its result before trying again.";
  }
  if (code === "credential_invalid")
    return "The assistant credential could not be accepted.";
  if (code === "model_not_configured")
    return "This agent is unavailable. Ask NyxBot to check its configuration.";
  if (code === "model_unavailable")
    return "This agent is unavailable. Ask NyxBot to check its configuration.";
  if (code === "model_interrupted")
    return "The response was interrupted. Some actions may already have completed — check them before retrying.";
  if (code === "session_busy")
    return "The assistant session is busy. Try again shortly.";
  if (code === "queue_timeout")
    return "Your other NyxBot replies were still running, so this message was not started. Send it again.";
  if (code === "capacity_exceeded") {
    const waited =
      capacityWaitMs != null && capacityWaitMs > 0
        ? ` (waited ${formatCapacityWait(capacityWaitMs)})`
        : "";
    return `NyxBot is at capacity right now${waited}. Try again shortly.`;
  }
  if (code === "turn_lost")
    return "This turn was interrupted; check which actions completed before retrying.";
  if (code === "internal_error")
    return "NyxBot hit an internal error on this turn. Check which actions completed before retrying.";
  if (code === "assistant_unavailable")
    return "The assistant could not complete this turn. Try again.";
  if (code === "first_byte_timeout")
    return "The assistant did not start responding in time. Try again.";
  if (code === "invalid_stream")
    return "The assistant stream ended unexpectedly. Try again.";
  if (code === "output_too_large")
    return "The assistant reached its response limit.";
  if (code === "guest_model_route_required")
    return "This assistant isn't available to guests yet.";
  if (code === "nyxid_error")
    return "NyxID could not authorize the assistant operation.";
  if (code === "stale_response")
    return "The assistant context changed. Start a new turn.";
  if (code === "context_unavailable")
    return "The assistant context could not be restored. Try a new turn.";
  return "The assistant could not complete this turn.";
}

export function contextResetCopy(reason: string | null | undefined): string {
  switch (reason) {
    case "turn_failed":
      return CONTEXT_RESET_NOTICE;
    case "instructions_changed":
      return "Instructions changed; the assistant context was refreshed.";
    case "model_changed":
      return "The agent configuration changed; the assistant context was refreshed.";
    case "session_lost":
      return "The assistant session expired; a fresh context will be used.";
    case "audience_changed":
      return "The conversation audience changed; the assistant context was refreshed.";
    case "guest_turn":
      return "This guest turn uses a separate assistant context.";
    case "stopped":
      return "The turn was stopped, so the assistant continues in a fresh context with a recap. Check what the stopped turn finished before rerunning it.";
    case "credential_replaced":
      return "The assistant's key was replaced, so it continues in a fresh context with a recap of this chat.";
    case "user_reset":
      return "You reset the working context.";
    default:
      return CONTEXT_RESET_NOTICE;
  }
}

export function toolFailureCopy(
  failure: { kind: string; status?: number | null } | null | undefined,
): string {
  switch (failure?.kind) {
    case "service_status":
      return failure.status == null
        ? "The service returned an error. No HTTP status is available."
        : `The service responded with HTTP ${failure.status}.`;
    case "refused":
      return "NyxID refused this call.";
    case "internal":
      return "NyxID could not complete this call.";
    case "tool_error":
      return "The tool reported an error.";
    default:
      return "This step did not complete. No additional error details are available.";
  }
}

export function formatElapsed(seconds: number): string {
  if (seconds < 60) return `${seconds}s`;
  return `${Math.floor(seconds / 60)}:${String(seconds % 60).padStart(2, "0")}`;
}

export function recoveryReason(reason: string): string {
  switch (reason) {
    case "turn_active":
      return "Another turn is active. Check its result before retrying.";
    case "already_retried":
      return "This reply was already retried. Review the newer result.";
    case "agent_destroyed":
      return "This agent's threads are read-only.";
    case "attachment_expired":
      return "An original attachment is no longer available.";
    case "activity_outcome_uncertain":
      return "A tool may have taken effect. Check its result before another attempt.";
    case "activity_evidence_incomplete":
    case "lineage_incomplete":
      return "Completed actions cannot be verified for this retry.";
    case "input_unavailable":
      return "The original request is no longer available.";
    case "status_unavailable":
      return "Retry availability could not be checked. Check again before retrying.";
    default:
      return "This reply cannot be retried automatically.";
  }
}

export function deliveryText(
  delivery: NonNullable<
    NonNullable<AssistantMessage["turn_metadata"]>["channel_delivery"]
  >,
): string {
  if (
    delivery.status === "interrupted_prefix" &&
    delivery.heard_prefix_matches_sent
  ) {
    return `Caller interrupted after a verified prefix of ${String(delivery.heard_prefix_chars ?? 0)} characters. The remaining answer is unconfirmed.`;
  }
  if (delivery.status === "delivered")
    return "Provider confirmed message delivery.";
  if (delivery.status === "failed")
    return "Provider reported delivery failure.";
  return "Generated reply; caller hearing or message delivery is unconfirmed.";
}

export function elapsedLabel(
  startedAt: string | undefined,
  endedAt?: string | number | null,
  now = Date.now(),
): string | undefined {
  const from = Date.parse(startedAt ?? "");
  const to =
    typeof endedAt === "number" ? endedAt : endedAt ? Date.parse(endedAt) : now;
  if (!Number.isFinite(from) || !Number.isFinite(to)) return undefined;
  const seconds = Math.max(0, Math.floor((to - from) / 1000));
  return seconds < 3 ? undefined : formatElapsed(seconds);
}
export function diagnosticId(message: AssistantMessage): string {
  const detail = message.error_detail;
  if (!detail) return "";
  const failure = message.turn_metadata?.failure_reason;
  return [
    `turn ${detail.turn_id}`,
    detail.attempts != null ? `attempts ${detail.attempts}` : null,
    detail.capacity_wait_ms != null
      ? `capacity_wait_ms ${detail.capacity_wait_ms}`
      : null,
    message.upstream_error_code
      ? `upstream ${message.upstream_error_code}`
      : null,
    failure
      ? `cause ${[failure.source, failure.stage, failure.code, failure.http_status != null ? `HTTP ${failure.http_status}` : null, failure.nyxid_error_code != null ? `NyxID ${failure.nyxid_error_code}` : null].filter(Boolean).join(" / ")}`
      : null,
    detail.response_id ? `response ${detail.response_id}` : null,
    detail.upstream_request_id ? `request ${detail.upstream_request_id}` : null,
  ]
    .filter(Boolean)
    .join(" / ");
}
