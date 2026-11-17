import type { RequestOptions } from "./authenticatedTransport";
import {
  notificationPageSchema,
  notificationPushPreferences,
  notificationPushPreferencesSchema,
  notificationReadBody,
  notificationSummarySchema,
  type NotificationKind,
  type NotificationPushPreferences,
  type NotificationReadScope,
} from "../../features/notifications/notificationContracts";

type JsonRequest = (path: string, options?: RequestOptions) => Promise<unknown>;

export const NOTIFICATIONS_PATH = "/assistant/nyxagent/notifications";
const SETTINGS_PATH = "/assistant/nyxagent/settings";
export const NOTIFICATION_PAGE_LIMIT = 30;
const MAX_LIMIT = 50;

export type NotificationListParams = {
  cursor?: string | null;
  limit?: number;
  unreadOnly?: boolean;
  /** Needs you: only decisions still waiting, seen or not. */
  pendingOnly?: boolean;
  kinds?: readonly NotificationKind[];
};

export function notificationListPath(params: NotificationListParams): string {
  const limit = Math.min(
    MAX_LIMIT,
    Math.max(1, Math.floor(params.limit ?? NOTIFICATION_PAGE_LIMIT)),
  );
  const query = new URLSearchParams({ limit: String(limit) });
  if (params.cursor) query.set("cursor", params.cursor);
  if (params.unreadOnly) query.set("unread_only", "true");
  if (params.pendingOnly) query.set("pending_only", "true");
  if (params.kinds?.length) query.set("kind", [...new Set(params.kinds)].join(","));
  return `${NOTIFICATIONS_PATH}?${query}`;
}

/** Human-only first-party routes; the mobile session token satisfies them. */
export function createNotificationsApi(request: JsonRequest) {
  return {
    async list(params: NotificationListParams = {}, signal?: AbortSignal) {
      return notificationPageSchema.parse(
        await request(notificationListPath(params), { method: "GET", signal }),
      );
    },
    async summary(signal?: AbortSignal) {
      return notificationSummarySchema.parse(
        await request(`${NOTIFICATIONS_PATH}/summary`, { method: "GET", signal }),
      );
    },
    async markRead(scope: NotificationReadScope) {
      await request(`${NOTIFICATIONS_PATH}/read`, {
        method: "POST",
        body: notificationReadBody(scope),
      });
    },
    async pushPreferences(signal?: AbortSignal) {
      return notificationPushPreferences(
        await request(SETTINGS_PATH, { method: "GET", signal }),
      );
    },
    /** Sends the complete six-switch object so one toggle cannot drop another. */
    async updatePushPreferences(push: NotificationPushPreferences) {
      const body = notificationPushPreferencesSchema.parse(push);
      return notificationPushPreferences(
        await request(SETTINGS_PATH, {
          method: "PUT",
          body: { notifications: { push: body } },
        }),
      );
    },
  };
}
export type NotificationsApi = ReturnType<typeof createNotificationsApi>;
