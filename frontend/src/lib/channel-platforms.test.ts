import { describe, expect, it } from "vitest";
import { CHANNEL_PLATFORMS, channelBotRegistrationPayload, editableChannelFields } from "./channel-platforms";

describe("channel platform fields", () => {
  it("keeps platform setup guidance in the descriptor", () => {
    for (const platform of ["lark", "feishu"] as const) {
      expect(CHANNEL_PLATFORMS[platform].setupNote).toEqual({
        title: "Lark webhook verification",
        text: "In Lark/Feishu Event Subscriptions, copy the Verification Token from Security settings. Encrypt Key is optional and should match the Encrypt Key field from the same panel if you enabled encrypted callbacks.",
      });
    }
    expect(CHANNEL_PLATFORMS.whatsapp.setupNote?.text).toContain("Phone Number ID");
    expect(CHANNEL_PLATFORMS.whatsapp.setupNote?.text).toContain("App Secret");
    expect(CHANNEL_PLATFORMS.telegram.setupNote).toBeUndefined();
  });
  it("does not submit hidden credentials after switching platform", () => {
    expect(channelBotRegistrationPayload({ platform: "whatsapp", label: " Support ", bot_token: " token ", phone_number_id: "1234", app_secret: "secret", app_id: "old-lark-app", encrypt_key: "old-key", verification_token: "old-token", public_key: "old-key" }))
      .toEqual({ platform: "whatsapp", label: "Support", bot_token: "token", phone_number_id: "1234", app_secret: "secret", target_org_id: undefined });
  });

  it("only exposes mutable credentials on the edit form", () => {
    expect(editableChannelFields("whatsapp").map((field) => field.name)).toEqual(["bot_token", "app_secret"]);
    expect(editableChannelFields("slack").map((field) => field.name)).toEqual(["app_secret"]);
    expect(editableChannelFields("telegram")).toEqual([]);
    expect(CHANNEL_PLATFORMS.whatsapp.fields.find((field) => field.name === "phone_number_id")?.label).toBe("Phone Number ID");
  });
});
