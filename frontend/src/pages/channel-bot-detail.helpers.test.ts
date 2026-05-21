import { describe, expect, it } from "vitest";
import type { ChannelBotStatus, ChannelPlatform } from "@/types/channels";
import {
  conversationTypeLabel,
  platformLabel,
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

describe("platformLabel", () => {
  it("labels telegram", () => {
    expect(platformLabel("telegram")).toBe("Telegram");
  });

  it("labels discord", () => {
    expect(platformLabel("discord")).toBe("Discord");
  });

  it("labels lark", () => {
    expect(platformLabel("lark")).toBe("Lark");
  });

  it("labels feishu", () => {
    expect(platformLabel("feishu")).toBe("Feishu");
  });

  it("labels slack", () => {
    expect(platformLabel("slack")).toBe("Slack");
  });

  it("echoes unknown platforms verbatim", () => {
    expect(platformLabel("matrix" as ChannelPlatform)).toBe("matrix");
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
