import type { CmaOperation } from "./assistantTypes";
export const isCmaOperationPending = (operation: CmaOperation) =>
  ["submitting", "waiting", "cancelling"].includes(operation.state);
export function cmaPresentation(operation: CmaOperation) {
  const { state, reason, status, kind } = operation;
  if (state === "cancelled")
    return { label: "Tracking stopped", tone: "muted" };
  if (state === "cancelling")
    return { label: "Stopping tracking", tone: "muted" };
  if (reason === "timeout")
    return { label: "Tracking timed out", tone: "warning" };
  if (["submission_uncertain", "submission_failed"].includes(reason ?? "")) {
    return { label: "Submission unconfirmed", tone: "warning" };
  }
  if (
    reason?.startsWith("result_") &&
    ["completed", "succeeded", "ready"].includes(status ?? "")
  ) {
    return {
      label: kind === "cma_response" ? "Completed" : "Agent ready",
      tone: "success",
    };
  }
  if (
    reason === "operation_failed" ||
    ["failed", "incomplete", "cancelled", "stopped", "deleted"].includes(
      status ?? "",
    )
  ) {
    return {
      label:
        status === "incomplete"
          ? "Incomplete"
          : status === "cancelled" || status === "stopped"
            ? "Stopped"
            : "Failed",
      tone: "error",
    };
  }
  if (reason && reason !== "completed") {
    return { label: "Status unavailable", tone: "warning" };
  }
  if (status === "completed" || reason === "completed") {
    return {
      label: kind === "cma_response" ? "Completed" : "Agent ready",
      tone: "success",
    };
  }
  if (
    status === "ready" &&
    kind === "cma_agent" &&
    isCmaOperationPending(operation)
  ) {
    return { label: "Agent ready", tone: "active" };
  }
  if (status === "succeeded" || status === "ready")
    return { label: "Agent ready", tone: "success" };
  if (
    [
      "waiting_approval",
      "connection_required",
      "awaiting_connection",
      "waiting_for_connection",
    ].includes(status ?? "")
  ) {
    return {
      label:
        status === "waiting_approval" ? "Needs approval" : "Needs connection",
      tone: "warning",
    };
  }
  if (status === "paused") return { label: "Paused", tone: "warning" };
  if (state === "submitting") return { label: "Submitting", tone: "active" };
  if (!status) return { label: "Awaiting status", tone: "active" };
  if (status === "queued") return { label: "Queued", tone: "active" };
  return {
    label: kind === "cma_provisioning" ? "Provisioning" : "Running",
    tone: "active",
  };
}
