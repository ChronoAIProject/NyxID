import type { RemoteMachine, RemoteMachineOperation } from "@/schemas/machines";

/** Current resource eligibility for a new owner action, separate from feedback. */
export function remoteMachineActionEligible(
  resource: RemoteMachine["resource_state"],
  action: "pause" | "resume" | "delete",
) {
  return action === "pause"
    ? resource === "ready" || resource === "degraded"
    : action === "resume"
      ? resource === "paused"
      : resource !== "deleting" && resource !== "deleted";
}

/** Selected-operation finality for display and duplicate-action prevention. */
export function remoteMachineLifecyclePending(
  machine: RemoteMachine,
  operationId: string | undefined,
  operation: RemoteMachineOperation | undefined,
  machineReadBehind: boolean,
) {
  if (machineReadBehind) return true;
  if (!operationId) return false;
  if (!operation) return true;
  if (operation.state === "failed") return false;
  const resourceResult =
    operation.action === "create" || operation.action === "resume"
      ? machine.resource_state === "ready"
      : operation.action === "pause"
        ? machine.resource_state === "paused"
        : operation.action === "delete"
          ? machine.resource_state === "deleted"
          : undefined;
  return resourceResult === undefined
    ? !operation.terminal_at && operation.state !== "succeeded"
    : operation.state !== "succeeded" || !resourceResult;
}
