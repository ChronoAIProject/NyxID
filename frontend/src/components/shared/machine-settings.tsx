import {
  useMachineSettings,
  useMachineUpdatePolicy,
} from "@/hooks/use-machines";
import type { MachineUpdateStatus } from "@/schemas/machines";
import type { NodeInfo } from "@/types/nodes";
import { MachineSettingsEditor } from "./machine-settings-editor";

/** Self-hosted write adapter. Remote presentation never mounts these mutations. */
export function MachineSettings({
  node,
  canManage,
  update,
}: {
  readonly node: NodeInfo;
  readonly canManage: boolean;
  readonly update?: MachineUpdateStatus;
}) {
  const settings = useMachineSettings(node.id);
  const updatePolicy = useMachineUpdatePolicy(node.id);
  if (!node.machine) return null;
  return (
    <MachineSettingsEditor
      key={node.id}
      machineId={node.id}
      ownerId={node.owner?.id}
      name={node.name}
      machine={node.machine}
      connected={node.is_connected}
      agentVersion={node.metadata?.agent_version}
      confirmation={node.machine_confirm}
      allowSharedLogins={node.allow_single_user_saved_logins}
      editable
      canManage={canManage}
      updateStatus={update}
      onSavePolicy={async (values) => {
        if (update && values.automatic !== update.automatic)
          await updatePolicy.mutateAsync(values.automatic);
        if (
          values.confirm !== (node.machine_confirm ?? "none") ||
          values.allow !== (node.allow_single_user_saved_logins ?? false)
        )
          await settings.mutateAsync({
            machine_confirm: values.confirm,
            allow_single_user_saved_logins: values.allow,
            acknowledge_single_user_risk: values.acknowledged,
          });
      }}
    />
  );
}
