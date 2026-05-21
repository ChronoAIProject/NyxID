import { describe, it, expect } from "vitest";
import {
  createChannelBotSchema,
  updateChannelBotSchema,
  createChannelConversationSchema,
  createDeviceConversationSchema,
  updateChannelConversationSchema,
  conversationPlatformSchema,
} from "./channels";

const UUID = "123e4567-e89b-12d3-a456-426614174000";

describe("createChannelBotSchema base validation", () => {
  it("accepts a minimal Telegram bot (no extra console fields required)", () => {
    expect(
      createChannelBotSchema.safeParse({
        platform: "telegram",
        bot_token: "123:abc",
        label: "support",
      }).success,
    ).toBe(true);
  });

  it("requires bot_token and label", () => {
    expect(
      createChannelBotSchema.safeParse({ platform: "telegram", bot_token: "", label: "x" })
        .success,
    ).toBe(false);
    expect(
      createChannelBotSchema.safeParse({ platform: "telegram", bot_token: "t", label: "" })
        .success,
    ).toBe(false);
  });
});

describe("createChannelBotSchema platform-specific superRefine", () => {
  it("requires app_id, app_secret, and verification_token for Lark/Feishu", () => {
    for (const platform of ["lark", "feishu"] as const) {
      const missing = createChannelBotSchema.safeParse({
        platform,
        bot_token: "t",
        label: "l",
      });
      expect(missing.success).toBe(false);
      if (!missing.success) {
        const paths = missing.error.issues.map((i) => i.path.join("."));
        expect(paths).toContain("app_id");
        expect(paths).toContain("app_secret");
        expect(paths).toContain("verification_token");
      }

      expect(
        createChannelBotSchema.safeParse({
          platform,
          bot_token: "t",
          label: "l",
          app_id: "cli_x",
          app_secret: "secret",
          verification_token: "vtok",
        }).success,
      ).toBe(true);
    }
  });

  it("treats whitespace-only console fields as missing for Lark", () => {
    const result = createChannelBotSchema.safeParse({
      platform: "lark",
      bot_token: "t",
      label: "l",
      app_id: "   ",
      app_secret: "secret",
      verification_token: "vtok",
    });
    expect(result.success).toBe(false);
    if (!result.success) {
      expect(result.error.issues.some((i) => i.path.join(".") === "app_id")).toBe(true);
    }
  });

  it("requires public_key for Discord", () => {
    expect(
      createChannelBotSchema.safeParse({ platform: "discord", bot_token: "t", label: "l" })
        .success,
    ).toBe(false);
    expect(
      createChannelBotSchema.safeParse({
        platform: "discord",
        bot_token: "t",
        label: "l",
        public_key: "pk",
      }).success,
    ).toBe(true);
  });

  it("requires app_secret (signing secret) for Slack", () => {
    expect(
      createChannelBotSchema.safeParse({ platform: "slack", bot_token: "t", label: "l" })
        .success,
    ).toBe(false);
    expect(
      createChannelBotSchema.safeParse({
        platform: "slack",
        bot_token: "t",
        label: "l",
        app_secret: "signing",
      }).success,
    ).toBe(true);
  });
});

describe("updateChannelBotSchema", () => {
  it("accepts an empty partial patch and rejects an over-long label", () => {
    expect(updateChannelBotSchema.safeParse({}).success).toBe(true);
    expect(
      updateChannelBotSchema.safeParse({ label: "a".repeat(129) }).success,
    ).toBe(false);
  });
});

describe("conversation schemas", () => {
  it("createChannelConversationSchema requires UUID bot + api key ids", () => {
    expect(
      createChannelConversationSchema.safeParse({
        channel_bot_id: UUID,
        agent_api_key_id: UUID,
      }).success,
    ).toBe(true);
    expect(
      createChannelConversationSchema.safeParse({
        channel_bot_id: "not-a-uuid",
        agent_api_key_id: UUID,
      }).success,
    ).toBe(false);
  });

  it("createDeviceConversationSchema requires a non-empty device channel id", () => {
    expect(
      createDeviceConversationSchema.safeParse({
        platform_conversation_id: "household-camera",
        agent_api_key_id: UUID,
      }).success,
    ).toBe(true);
    expect(
      createDeviceConversationSchema.safeParse({
        platform_conversation_id: "",
        agent_api_key_id: UUID,
      }).success,
    ).toBe(false);
  });

  it("updateChannelConversationSchema accepts an empty patch but validates api key id format", () => {
    expect(updateChannelConversationSchema.safeParse({}).success).toBe(true);
    expect(
      updateChannelConversationSchema.safeParse({ agent_api_key_id: "bad" }).success,
    ).toBe(false);
  });
});

describe("conversationPlatformSchema", () => {
  it("includes device but not slack (read-back set)", () => {
    expect(conversationPlatformSchema.safeParse("device").success).toBe(true);
    expect(conversationPlatformSchema.safeParse("slack").success).toBe(false);
  });
});
