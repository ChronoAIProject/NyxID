import type { ComponentProps } from "react";
import { AssistantPendingCredentialCreateDialog } from "./assistant-pending-credential-create-dialog";

export function AssistantNodeInjectCredentialDialog(
  props: Omit<
    ComponentProps<typeof AssistantPendingCredentialCreateDialog>,
    "mode"
  >,
) {
  return <AssistantPendingCredentialCreateDialog {...props} mode="inject" />;
}
