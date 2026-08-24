import type { ComponentProps } from "react";
import { AssistantApprovalToggleDialog } from "./assistant-approval-toggle-dialog";

export function AssistantApprovalDisableDialog(props: Omit<ComponentProps<typeof AssistantApprovalToggleDialog>, "mode">) {
  return <AssistantApprovalToggleDialog {...props} mode="disable" />;
}
