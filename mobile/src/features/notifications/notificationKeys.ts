import type { NotificationFilter } from "./notificationContracts";

/** Query-key prefix shared by every notification query; dependency-free so
 * push handling can invalidate it without loading the hook module. */
export const NOTIFICATIONS = ["workspace", "notifications"] as const;

export const notificationKeys = {
  summary: (epoch: number) => [...NOTIFICATIONS, "summary", epoch] as const,
  feed: (epoch: number, filter: NotificationFilter) =>
    [...NOTIFICATIONS, "feed", epoch, filter] as const,
  preferences: (epoch: number) =>
    [...NOTIFICATIONS, "preferences", epoch] as const,
};
