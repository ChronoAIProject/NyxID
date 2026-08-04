import type { ChannelBotStatus, ChannelPlatform } from "@/types/channels";

export function statusBadgeVariant(
  status: ChannelBotStatus,
): "success" | "warning" | "destructive" | "secondary" {
  switch (status) {
    case "active":
      return "success";
    case "pending":
    case "pending_webhook":
      return "warning";
    case "failed":
      return "destructive";
    case "invalid":
      return "secondary";
    default:
      return "secondary";
  }
}

export function statusLabel(status: ChannelBotStatus): string {
  switch (status) {
    case "active":
      return "Active";
    case "pending":
      return "Pending";
    case "pending_webhook":
      return "Pending Webhook";
    case "failed":
      return "Failed";
    case "invalid":
      return "Invalid";
    default:
      return status;
  }
}

export function platformLabel(platform: ChannelPlatform): string {
  switch (platform) {
    case "telegram":
      return "Telegram";
    case "discord":
      return "Discord";
    case "lark":
      return "Lark";
    case "feishu":
      return "Feishu";
    case "slack":
      return "Slack";
    default:
      return platform;
  }
}

export function conversationTypeLabel(t: string): string {
  switch (t) {
    case "private":
      return "Private";
    case "group":
      return "Group";
    case "channel":
      return "Channel";
    default:
      return t;
  }
}
