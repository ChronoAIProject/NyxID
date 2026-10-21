import type { NotificationSettings } from "../../lib/api/types";

export function approvalDeliveryWarning(settings: Partial<Pick<NotificationSettings, "approval_web_available" | "approval_suspended">> | undefined): string {
  if (settings?.approval_suspended) {
    return "This server reports approval protection is suspended without notifications. Keep a working notification channel enabled.";
  }
  if (settings?.approval_web_available !== true || settings?.approval_suspended !== false) {
    return "Web-only approval protection is unconfirmed on this server. Keep a working notification channel enabled.";
  }
  return "Approval remains required when notifications are unavailable. Review pending requests in Approval History on the web before they expire.";
}
