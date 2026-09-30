import { describe, expect, it } from "vitest";
import { assistantModalLinkTarget } from "./assistant-link-target";

const ORIGIN = "https://nyxid.example";

describe("assistantModalLinkTarget", () => {
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
