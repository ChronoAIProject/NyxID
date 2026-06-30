import type { ChannelBotStatus, ChannelPlatform } from "@/types/channels";
import { getStatusMeta } from "@/lib/status-contract";

/**
 * Reads from `STATUS_REGISTRY.channel_bot` so the variant + label for each
 * status live in exactly one place. Falls back to "secondary"/raw key for
 * unknown statuses so the UI never blows up if the backend ships a new
 * value before the registry is updated. Wave B B.4 deferred migration,
 * landed in Wave C C.1 alongside the canon sweep.
 *
 * Return type is intentionally narrower than the full `StatusVariant`
 * because channel_bot statuses today never need `info`/`pending`/`outline`
 * variants — keeping the contract tight stops consumers from accidentally
 * widening the variant pool.
 */
export function statusBadgeVariant(
  status: ChannelBotStatus,
): "success" | "warning" | "destructive" | "secondary" {
  const variant = getStatusMeta("channel_bot", status)?.variant;
  switch (variant) {
    case "success":
    case "warning":
    case "destructive":
    case "secondary":
      return variant;
    default:
      return "secondary";
  }
}

export function statusLabel(status: ChannelBotStatus): string {
  return getStatusMeta("channel_bot", status)?.label ?? status;
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
