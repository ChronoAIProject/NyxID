import { Badge } from "@/components/ui/badge";
import {
  SINGLE_USER_SHELL_WARNING,
  type MachineProfile,
} from "@/schemas/machines";

export function MachineIsolationBadge({
  machine,
}: {
  readonly machine: MachineProfile;
}) {
  if (!machine.shell || machine.commands_isolated === true) return null;
  return (
    <Badge variant="warning">
      {machine.commands_isolated === false
        ? "Not isolated"
        : "Isolation unknown: update to check"}
    </Badge>
  );
}

export function MachineIsolationDetails({
  machine,
}: {
  readonly machine: MachineProfile;
}) {
  if (!machine.shell || machine.commands_isolated !== false) return null;
  return (
    <div className="space-y-1 text-muted-foreground">
      <p>Agent commands can read this node’s stored credentials and token.</p>
      <details>
        <summary className="cursor-pointer text-foreground">Details</summary>
        <p className="pt-2">{SINGLE_USER_SHELL_WARNING}</p>
      </details>
    </div>
  );
}
