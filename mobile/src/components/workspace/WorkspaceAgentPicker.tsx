import { useNotificationIndicator } from "../../features/notifications/useNotifications";
import { WorkspaceChatList, type WorkspaceChatListProps } from "./WorkspaceChatList";
import { useWorkspaceRecents } from "./useWorkspaceRecents";

/** The task-local picker shares the sidebar's live Recents query and markers. */
export function WorkspaceAgentPicker(props: Omit<WorkspaceChatListProps, "mode" | "recentRows" | "unread">) {
  const recents = useWorkspaceRecents(props.visible);
  const notifications = useNotificationIndicator(recents.active);
  return <WorkspaceChatList {...props} mode="picker" recentRows={recents.rows} unread={notifications.markers} />;
}
