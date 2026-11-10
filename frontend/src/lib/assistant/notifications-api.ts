import { assistantJson } from "@/lib/assistant/assistant-http";
import {
  NOTIFICATION_FILTER_KINDS,
  NOTIFICATION_READ_MAX_IDS,
  notificationListSchema,
  notificationSummarySchema,
  type NotificationFilter,
  type NotificationReadRequest,
} from "@/schemas/assistant-notifications";

export const NOTIFICATIONS_PATH = "/assistant/nyxagent/notifications";
/** One page of the feed; the server caps requests at 50. */
export const NOTIFICATIONS_PAGE_SIZE = 30;

export interface NotificationListParams {
  readonly cursor?: string;
  readonly filter?: NotificationFilter;
  readonly unreadOnly?: boolean;
  readonly limit?: number;
}

export function notificationListQuery(params: NotificationListParams): string {
  const query = new URLSearchParams();
  query.set("limit", String(params.limit ?? NOTIFICATIONS_PAGE_SIZE));
  if (params.cursor) query.set("cursor", params.cursor);
  if (params.unreadOnly) query.set("unread_only", "true");
  const kinds = params.filter ? NOTIFICATION_FILTER_KINDS[params.filter] : [];
  if (kinds.length) query.set("kind", kinds.join(","));
  return query.toString();
}

/** The notifications feed. The server owns read state; the browser only reports what was seen. */
export const notificationsApi = {
  async list(params: NotificationListParams = {}) {
    return notificationListSchema.parse(
      await assistantJson(`${NOTIFICATIONS_PATH}?${notificationListQuery(params)}`),
    );
  },
  async summary() {
    return notificationSummarySchema.parse(await assistantJson(`${NOTIFICATIONS_PATH}/summary`));
  },
  /** Idempotent; a 204 or a small JSON receipt are both accepted. */
  async markRead(body: NotificationReadRequest) {
    if ("ids" in body && body.ids.length > NOTIFICATION_READ_MAX_IDS) {
      throw new Error(`At most ${String(NOTIFICATION_READ_MAX_IDS)} notifications can be marked at once.`);
    }
    await assistantJson(`${NOTIFICATIONS_PATH}/read`, { method: "POST", body });
  },
};
