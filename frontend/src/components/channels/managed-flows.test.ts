import { describe, expect, it } from "vitest";
import {
  CHANNEL_PLATFORMS,
  managedConnectPlatform,
} from "@/lib/channel-platforms";
import {
  createChannelBotSchema,
  conversationPlatformSchema,
} from "@/schemas/channels";
import {
  managedBootstrapSchema,
  oauthConnectionCompleteSchema,
} from "@/schemas/channel-managed";
import { MANAGED_FLOW_COMPONENTS } from "./managed-flows";
import {
  ManagedOAuthConnect,
  ManagedOAuthDetail,
} from "./managed-oauth-connect";
import { ManagedWhatsApp } from "./managed-whatsapp";

describe("managed flow registry", () => {
  it("resolves every descriptor through a flow component", () => {
    for (const descriptor of Object.values(CHANNEL_PLATFORMS)) {
      if (descriptor.managedFlow)
        expect(
          MANAGED_FLOW_COMPONENTS[descriptor.managedFlow].Connect,
        ).toBeTypeOf("function");
    }
    expect(MANAGED_FLOW_COMPONENTS.oauth_connection.Connect).toBe(
      ManagedOAuthConnect,
    );
    expect(MANAGED_FLOW_COMPONENTS.oauth_connection.Detail).toBe(
      ManagedOAuthDetail,
    );
    expect(MANAGED_FLOW_COMPONENTS.meta_embedded_signup.Connect).toBe(
      ManagedWhatsApp,
    );
  });
  it("supports X deep links and requires no BYO fields", () => {
    expect(managedConnectPlatform("x")).toBe("x");
    expect(managedConnectPlatform("whatsapp")).toBe("whatsapp");
    expect(managedConnectPlatform("telegram")).toBeUndefined();
    expect(managedConnectPlatform("__proto__")).toBeUndefined();
    expect(CHANNEL_PLATFORMS.x.fields).toEqual([]);
    expect(CHANNEL_PLATFORMS.x.managedOnly).toBe(true);
    expect(
      createChannelBotSchema.safeParse({
        platform: "x",
        label: "Support",
        bot_token: "",
      }).success,
    ).toBe(true);
    expect(
      createChannelBotSchema.safeParse({
        platform: "telegram",
        label: "Support",
        bot_token: "",
      }).success,
    ).toBe(false);
  });
  it("validates OAuth completion and bootstrap independently of embedded signup", () => {
    expect(
      managedBootstrapSchema.parse({
        available: true,
        flow: "oauth_connection",
        provider_slug: "twitter",
        required_scopes: ["dm.read"],
      }).provider_slug,
    ).toBe("twitter");
    expect(
      oauthConnectionCompleteSchema.safeParse({
        connection_id: "bad",
        label: "Support",
      }).success,
    ).toBe(false);
    expect(
      oauthConnectionCompleteSchema.safeParse({
        connection_id: "11111111-1111-4111-8111-111111111111",
        label: "Support",
        code: "unwanted",
      }).success,
    ).toBe(false);
    expect(conversationPlatformSchema.safeParse("x").success).toBe(true);
  });
});
