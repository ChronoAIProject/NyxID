import { describe, expect, it } from "vitest";
import { channelPlatformViews, channelBotRegistrationPayload, editableChannelFields, managedConnectPlatform, platformView } from "./channel-platforms";
import { platformFixtures } from "@/test/fixtures/channel-platforms";
const platforms = channelPlatformViews(platformFixtures);

describe("channel catalog presentation", () => {
  it("uses API names and flags, including unknown or disabled descriptors", () => {
    expect(platforms.telegram!.label).toBe("Telegram bot token");
    expect(platforms["telegram-new"]!.label).toBe("Telegram");
    expect(platforms.x!.managedOnly).toBe(true);
    expect(platformView(undefined, "future").enabled).toBe(false);
    expect(managedConnectPlatform("__proto__")).toBeUndefined();
  });
  it("preserves API field hints and setup instructions without platform-specific copy", () => {
    const base = platformFixtures[0]!;
    const view = platformView({
      ...base,
      registration: {
        ...base.registration,
        fields: [{ ...base.registration.fields[0]!, hint: "Copy this from the developer console." }],
        setup_instructions: ["Grant permissions.", "Enable events."],
      },
    });
    expect(view.fields[0]!.hint).toBe("Copy this from the developer console.");
    expect(view.setupNote?.text).toBe("Grant permissions. Enable events.");
    expect(platformView(base).fields[0]!.hint).toBeNull();
  });
  it("does not submit credentials hidden after switching platform", () => {
    const descriptor = platformFixtures.find((p) => p.platform === "whatsapp")!;
    expect(channelBotRegistrationPayload({ platform: "whatsapp", label: " Support ", bot_token: " token ", phone_number_id: "1234", app_secret: "secret", app_id: "hidden", encrypt_key: "hidden" }, descriptor))
      .toEqual({ platform: "whatsapp", label: "Support", bot_token: "token", phone_number_id: "1234", app_secret: "secret", target_org_id: undefined });
  });
  it("shows only adapter-declared patchable fields", () => {
    expect(editableChannelFields(platforms.whatsapp!).map((f) => f.name)).toEqual(["bot_token", "app_secret"]);
    expect(editableChannelFields(platforms.telegram!)).toEqual([]);
  });
});
