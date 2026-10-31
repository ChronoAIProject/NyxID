import type { MachineAccess } from "@/schemas/machine-access";
import { hasPendingMachineAuthority } from "@/lib/machine-access";

/** These observations describe Agent authority, never hosted configuration. */
export function MachineAuthorizationStatus({
  machine,
}: {
  readonly machine: MachineAccess;
}) {
  const executionPending =
    machine.execution_revocation_pending ?? machine.revocation_pending;
  const deadline = machine.revocation_expires_at
    ? new Date(machine.revocation_expires_at)
    : undefined;
  const deadlineLabel =
    deadline && Number.isFinite(deadline.getTime())
      ? deadline.toLocaleString()
      : undefined;
  const settled =
    !hasPendingMachineAuthority(machine) &&
    !machine.revocation_diagnostic &&
    machine.revocation_pending === false &&
    machine.delivery_pending === false &&
    machine.quarantine_pending === false;
  return (
    <section
      aria-label="Agent authorization status"
      className="space-y-2 text-12 text-muted-foreground"
    >
      <p>Saved Agent assignment revision {machine.assignment_revision} · policy revision {machine.revision}</p>
      {machine.revocation_initialization_pending ? (
        <p role="status">
          Checking when earlier commands will stop. The execution authority
          deadline is not yet confirmed. Refresh status in a few seconds.
        </p>
      ) : executionPending ? (
        <p role="status">
          Earlier commands are stopping.
          {deadlineLabel
            ? ` Their execution authority expires by ${deadlineLabel}.`
            : " Reconnect and refresh status to check revocation."}{" "}
          An authority deadline does not prove physical runtime stop or undo
          completed external effects.
        </p>
      ) : machine.execution_revocation_pending === false &&
        machine.delivery_pending ? (
        <p role="status">
          Earlier execution authority has expired. Expiry does not confirm
          physical runtime stop or undo completed external effects.
        </p>
      ) : null}
      {machine.delivery_pending ? (
        <p role="status">
          Waiting for machine acknowledgement of the updated Agent authority.
          Reconnect the machine and refresh status; elapsed time does not
          confirm delivery.
        </p>
      ) : null}
      {machine.quarantine_pending ? (
        <p role="status" className="text-warning">
          Browser cleanup is pending. Reconnect the machine and refresh status.
          New browser work stays blocked until cleanup finishes.
        </p>
      ) : null}
      {machine.revocation_diagnostic ? (
        <p role="status" className="text-warning">
          {machine.revocation_diagnostic === "authority_timing_upgrade_required"
            ? "Update and reconnect this machine to confirm permission changes."
            : "Recovery status needs administrator attention. Earlier commands are still marked as stopping; check machine diagnostics before continuing."}
        </p>
      ) : null}
      {settled ? (
        <p role="status">No pending Agent authority changes reported.</p>
      ) : !hasPendingMachineAuthority(machine) &&
        !machine.revocation_diagnostic ? (
        <p role="status">
          Agent authority application status is not reported. Refresh machine
          status to check.
        </p>
      ) : null}
    </section>
  );
}
