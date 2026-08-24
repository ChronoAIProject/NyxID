import type { ComponentProps } from "react";
import { AssistantApprovalToggleDialog } from "./assistant-approval-toggle-dialog";

export function AssistantApprovalEnableDialog(props: Omit<ComponentProps<typeof AssistantApprovalToggleDialog>, "mode">) {
  return <AssistantApprovalToggleDialog {...props} mode="enable" />;
}
