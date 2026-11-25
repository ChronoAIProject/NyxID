import { describe, expect, it } from "vitest";
import type { ChannelBotStatus } from "@/types/channels";
import {
  channelDeleteMessage,
  conversationTypeLabel,
  statusBadgeVariant,
  statusLabel,
} from "./channel-bot-detail.helpers";

describe("statusBadgeVariant", () => {
  it("maps active to success", () => {
    expect(statusBadgeVariant("active")).toBe("success");
  });

  it("maps pending to warning", () => {
    expect(statusBadgeVariant("pending")).toBe("warning");
  });

  it("maps pending_webhook to warning", () => {
    expect(statusBadgeVariant("pending_webhook")).toBe("warning");
  });

  it("maps failed to destructive", () => {
    expect(statusBadgeVariant("failed")).toBe("destructive");
  });

  it("maps invalid to secondary", () => {
    expect(statusBadgeVariant("invalid")).toBe("secondary");
  });

  it("falls back to secondary for unknown statuses", () => {
    expect(statusBadgeVariant("retired" as ChannelBotStatus)).toBe("secondary");
  });
});

describe("statusLabel", () => {
  it("labels active", () => {
    expect(statusLabel("active")).toBe("Active");
  });

  it("labels pending", () => {
    expect(statusLabel("pending")).toBe("Pending");
  });

  it("labels pending_webhook", () => {
    expect(statusLabel("pending_webhook")).toBe("Pending Webhook");
  });

  it("labels failed", () => {
    expect(statusLabel("failed")).toBe("Failed");
  });

  it("labels invalid", () => {
    expect(statusLabel("invalid")).toBe("Invalid");
  });

  it("echoes unknown statuses verbatim", () => {
    expect(statusLabel("archived" as ChannelBotStatus)).toBe("archived");
  });
});

describe("conversationTypeLabel", () => {
  it("labels private", () => {
    expect(conversationTypeLabel("private")).toBe("Private");
  });

  it("labels group", () => {
    expect(conversationTypeLabel("group")).toBe("Group");
  });

  it("labels channel", () => {
    expect(conversationTypeLabel("channel")).toBe("Channel");
  });

  it("echoes unknown conversation types verbatim", () => {
    expect(conversationTypeLabel("device")).toBe("device");
  });
});

it.each([
  [{ local_delivery_stopped: true, webhook_cleanup: "failed" }, "Inbox delivery stopped. NyxID will retry removing the subscription."],
  [{ local_delivery_stopped: true, webhook_cleanup: "removed" }, "Inbox delivery stopped. Subscription cleanup for this inbox is complete."],
  [{ webhook_cleanup: "failed" }, "Bot deleted. Remove its remaining email subscription in the Aurinko dashboard."],
  [undefined, "Bot deleted"],
] as const)("reports the deletion result for %j", (result, expected) => {
  expect(channelDeleteMessage(result)).toBe(expected);
});
