import { MachineToolCard } from "../machines/MachineToolCard";
import { AssistantAttachmentView } from "../assistant/AssistantAttachmentView";
import type { GroupMessage } from "./groupContracts";

/** Crew notice and agent rows use the same authenticated receipt/attachment path. */
export function GroupMachineReceipts({
  message,
  groupId,
  foreground,
}: {
  message: GroupMessage;
  groupId?: string;
  foreground?: string;
}) {
  return (
    <>
      {message.activities
        .filter((a) => a.machine)
        .map((a) => (
          <MachineToolCard
            key={a.id}
            receipt={a.machine!}
            groupId={groupId}
            attachments={message.attachments}
          />
        ))}
      {message.attachments
        .filter(
          (a) =>
            !message.activities.some(
              (tool) => tool.machine?.screenshot_id === a.id,
            ),
        )
        .map((a) => (
          <AssistantAttachmentView
            key={a.id}
            attachment={a}
            groupId={groupId}
            captionColor={foreground}
          />
        ))}
    </>
  );
}
