import type { MachineAccess } from "@/schemas/machine-access";

export function hasPendingMachineAuthority(machine: MachineAccess): boolean {
  return Boolean(
    machine.revocation_pending ||
    machine.delivery_pending ||
    machine.execution_revocation_pending ||
    machine.revocation_initialization_pending ||
    machine.quarantine_pending,
  );
}

export function hasPendingMachineApplication(machine: MachineAccess): boolean {
  return hasPendingMachineAuthority(machine) || Boolean(machine.machine_configuration && machine.machine_configuration.state !== "applied");
}
