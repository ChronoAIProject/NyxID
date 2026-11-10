import {
  notificationIdFromPushData,
  notificationKindFromPushData,
} from "../../features/notifications/notificationPresentation";
import type { NotificationKind } from "../../features/notifications/notificationContracts";

export type NotificationPushSignal = {
  kind: NotificationKind | null;
  notificationId: string | null;
  deeplink: string | null;
};

function asNonEmptyString(value: unknown): string | null {
  if (typeof value !== "string") return null;
  const trimmed = value.trim();
  return trimmed.length > 0 ? trimmed : null;
}

/** NyxBot notification pushes carry `type`, `notification_id` and `deeplink`
 * (SPEC D9); approval pushes carry none of these and yield null. */
export function notificationPushSignalFromData(
  data: unknown,
): NotificationPushSignal | null {
  if (!data || typeof data !== "object") return null;
  const kind = notificationKindFromPushData(data);
  const notificationId = notificationIdFromPushData(data);
  if (!kind && !notificationId) return null;
  const payload = data as { deeplink?: unknown; deep_link?: unknown };
  return {
    kind,
    notificationId,
    deeplink:
      asNonEmptyString(payload.deeplink) ?? asNonEmptyString(payload.deep_link),
  };
}

type ResponseLike = {
  notification: {
    request: {
      content: { data?: unknown };
      trigger?: unknown;
    };
  };
};

/** A tapped notification, read from its content first, then the raw push. */
export function notificationPushSignalFromResponse(
  response: ResponseLike | null,
): NotificationPushSignal | null {
  if (!response) return null;
  const request = response.notification.request;
  const fromContent = notificationPushSignalFromData(request.content.data);
  if (fromContent) return fromContent;
  const trigger = request.trigger as
    | { type?: unknown; payload?: unknown; remoteMessage?: { data?: unknown } | null }
    | null
    | undefined;
  if (!trigger || typeof trigger !== "object" || trigger.type !== "push")
    return null;
  return (
    notificationPushSignalFromData(trigger.payload) ??
    notificationPushSignalFromData(trigger.remoteMessage?.data)
  );
}

/** True for a NyxBot notification push: the app routes its destination and
 * marks it read itself, so the generic link router must leave it alone. */
export function isAssistantNotificationResponse(
  response: ResponseLike | null,
): boolean {
  return !!notificationPushSignalFromResponse(response)?.notificationId;
}
