import { translateApp } from "../../i18n/appTranslation";
import type { CmaOperation } from "./assistantTypes";
export const isCmaOperationPending = (operation: CmaOperation) =>
  ["submitting", "waiting", "cancelling"].includes(operation.state);
export function cmaPresentation(operation: CmaOperation) {
  const { state, reason, status, kind } = operation;
  if (!["cma_response", "cma_agent", "cma_provisioning"].includes(kind))
    return { label: translateApp("Activity details unavailable in this app version."), tone: "muted" };
  if (state === "cancelled")
    return { label: translateApp("Tracking stopped"), tone: "muted" };
  if (state === "cancelling")
    return { label: translateApp("Stopping tracking"), tone: "muted" };
  if (reason === "timeout")
    return { label: translateApp("Tracking timed out"), tone: "warning" };
  if (["submission_uncertain", "submission_failed"].includes(reason ?? "")) {
    return { label: translateApp("Submission unconfirmed"), tone: "warning" };
  }
  if (
    reason?.startsWith("result_") &&
    ["completed", "succeeded", "ready"].includes(status ?? "")
  ) {
    return {
      label: kind === "cma_response" ? translateApp("Completed")
          : translateApp("Agent ready"),
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
          ? translateApp("Incomplete")
          : status === "cancelled" || status === "stopped"
            ? translateApp("Stopped")
            : translateApp("Failed"),
      tone: "error",
    };
  }
  if (reason && reason !== "completed") {
    return { label: translateApp("Status unavailable"), tone: "warning" };
  }
  if (status === "completed" || reason === "completed") {
    return {
      label: kind === "cma_response" ? translateApp("Completed")
          : translateApp("Agent ready"),
      tone: "success",
    };
  }
  if (
    status === "ready" &&
    kind === "cma_agent" &&
    isCmaOperationPending(operation)
  ) {
    return { label: translateApp("Agent ready"), tone: "active" };
  }
  if (status === "succeeded" || status === "ready")
    return { label: translateApp("Agent ready"), tone: "success" };
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
        status === "waiting_approval" ? translateApp("Needs approval")
          : translateApp("Needs connection"),
      tone: "warning",
    };
  }
  if (status === "paused") return { label: translateApp("Paused"), tone: "warning" };
  if (state === "submitting") return { label: translateApp("Submitting"), tone: "active" };
  if (!status) return { label: translateApp("Awaiting status"), tone: "active" };
  if (status === "queued") return { label: translateApp("Queued"), tone: "active" };
  return {
    label: kind === "cma_provisioning" ? translateApp("Provisioning")
        : translateApp("Running"),
    tone: "active",
  };
}
