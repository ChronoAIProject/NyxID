import { describe, expect, it } from "vitest";
import { assistantModalLinkTarget } from "./assistant-link-target";

const ORIGIN = "https://nyxid.example";

describe("assistantModalLinkTarget", () => {
  it("keeps Telegram request recovery and assistant workspace journeys in the app", () => {
    const id = "11111111-1111-4111-8111-111111111111";
    expect(assistantModalLinkTarget(`/channel-bots?connect=telegram-new&request_id=${id}`, ORIGIN)).toMatchObject({ kind: "channel-bot", platform: "telegram-new", requestId: id });
    expect(assistantModalLinkTarget(`/assistant?agent=${id}&details=grants`, ORIGIN)).toMatchObject({ kind: "workspace" });
    for (const href of ["/assistant/automations", "/assistant/automations?setup=bounded-setup", "/assistant/machines", "/assistant/machines?tab=logins"]) expect(assistantModalLinkTarget(href, ORIGIN)).toMatchObject({ kind: "workspace" });
    for (const href of ["/channel-bots?connect=telegram-new&request_id=invalid", `/channel-bots?connect=telegram-new&request_id=${id}&request_id=${id}`, "/assistant/machines?return_to=https://evil.example", `/assistant?agent=${id}&details=invalid`, "https://evil.example/assistant/automations"]) expect(assistantModalLinkTarget(href, ORIGIN)).toBeNull();
  });
  it("preserves hosted bot setup tokens and accepts the agent-key creation journey", () => {
    expect(assistantModalLinkTarget("/connect/bot/nyx_bcl_example-123", ORIGIN)).toEqual({ kind: "channel-connect", href: `${ORIGIN}/connect/bot/nyx_bcl_example-123`, token: "nyx_bcl_example-123" });
    expect(assistantModalLinkTarget("/keys?tab=nyxid&action=create-key", ORIGIN)).toEqual({ kind: "agent-key", href: `${ORIGIN}/keys?tab=nyxid&action=create-key` });
    for (const href of [
      "https://evil.example/connect/bot/nyx_bcl_example", "/connect/bot/not-a-token",
      "/connect/bot/nyx_bcl_example?callback=https://evil.example", "/connect/bot/nyx_bcl_example#secret",
      "/connect/bot/nyx_bcl_example/extra", "/connect/bot/nyx_bcl_",
      "https://user:secret@nyxid.example/connect/bot/nyx_bcl_example",
      "https://evil.example/keys?tab=nyxid&action=create-key", "/keys?tab=nyxid&action=create-key&scope=admin",
      "/keys?tab=nyxid&action=create-key&tab=nyxid", "/keys?tab=nyxid&action=delete-key",
    ]) expect(assistantModalLinkTarget(href, ORIGIN)).toBeNull();
  });
  it("classifies hosted connector links and preserves their token", () => {
    expect(
      assistantModalLinkTarget(`${ORIGIN}/connect/nyx_clk_abc-123`, ORIGIN),
    ).toEqual({
      kind: "connect",
      href: `${ORIGIN}/connect/nyx_clk_abc-123`,
      token: "nyx_clk_abc-123",
    });
  });

  it("classifies same-origin channel setup links with bounded search", () => {
    expect(
      assistantModalLinkTarget(
        `${ORIGIN}/channel-bots/connect/telegram?label=Support%20bot`,
        ORIGIN,
      ),
    ).toEqual({
      kind: "channel-bot",
      href: `${ORIGIN}/channel-bots/connect/telegram?label=Support+bot`,
      platform: "telegram",
      search: "?label=Support+bot",
    });
  });

  it("rejects external, ambiguous, credentialed, and fragmented links", () => {
    for (const href of [
      "https://evil.example/connect/nyx_clk_abc",
      `${ORIGIN}/connect/nyx_clk_abc?next=/admin`,
      `${ORIGIN}/channel-bots/connect`,
      `${ORIGIN}/channel-bots/connect/telegram/extra`,
      `${ORIGIN}/channel-bots/connect/%2Fadmin`,
      `https://user:secret@nyxid.example/channel-bots/connect/telegram`,
      `${ORIGIN}/channel-bots/connect/telegram#credential`,
    ]) {
      expect(assistantModalLinkTarget(href, ORIGIN)).toBeNull();
    }
  });

  it("drops untrusted channel credential query fields before opening setup", () => {
    expect(
      assistantModalLinkTarget(
        `${ORIGIN}/channel-bots/connect/telegram?label=Support&bot_token=secret`,
        ORIGIN,
      ),
    ).toEqual({
      kind: "channel-bot",
      href: `${ORIGIN}/channel-bots/connect/telegram?label=Support`,
      platform: "telegram",
      search: "?label=Support",
    });
  });
});

it("classifies only same-origin account links with panel-owned query keys", () => {
  expect(assistantModalLinkTarget("/assistant?panel=settings&panelTab=security", ORIGIN)).toEqual({ kind: "account-panel", href: `${ORIGIN}/assistant?panel=settings&panelTab=security`, panel: "settings", params: { panelTab: "security" } });
  expect(assistantModalLinkTarget(`${ORIGIN}/assistant?panel=billing&panelTab=usage&panelServices=%5B%22a%22%5D`, ORIGIN)).toMatchObject({ kind: "account-panel", panel: "billing", params: { panelTab: "usage", panelServices: ["a"] } });
  expect(assistantModalLinkTarget("/assistant?panel=nyxbot", ORIGIN)).toMatchObject({ kind: "account-panel", panel: "nyxbot", params: {} });
  expect(assistantModalLinkTarget("/assistant?panel=notifications", ORIGIN)).toMatchObject({ kind: "account-panel", panel: "notifications", params: {} });
  expect(assistantModalLinkTarget("/assistant?panel=notifications&panelTab=needs_you", ORIGIN)).toMatchObject({ kind: "account-panel", panel: "notifications", params: { panelTab: "needs_you" } });
  // An unknown filter and the implicit All both open the unfiltered feed.
  expect(assistantModalLinkTarget("/assistant?panel=notifications&panelTab=everything", ORIGIN)).toMatchObject({ kind: "account-panel", panel: "notifications", params: {} });
  for (const href of ["https://evil.example/assistant?panel=settings", "https://user:secret@nyxid.example/assistant?panel=settings", "/assistant?panel=settings#x", "/assistant?panel=settings&c=chat", "/assistant?panel=settings&tab=security", "/assistant?panel=invalid", "/assistant?panel=billing&panelServices=invalid", "/assistant/settings?panel=settings"]) {
    expect(assistantModalLinkTarget(href, ORIGIN)).toBeNull();
  }
});
