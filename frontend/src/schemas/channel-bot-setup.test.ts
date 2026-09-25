import { defaultParseSearch } from "@tanstack/react-router";
import { describe, expect, it } from "vitest";
import { channelBotSetupRewrite } from "./channel-bot-setup";

describe("channel setup URL values", () => {
  it.each([
    "90071992547409931234",
    "000123",
    "1e3",
    "1.00",
    "true",
    "false",
    "null",
    "[]",
    '{"id":1}',
    "token+/=&value",
  ])("preserves %s as text before router parsing", (value) => {
    const url = new URL(
      "https://nyxid.example/channel-bots/connect/future-chat",
    );
    url.searchParams.set("field1", value);
    const rewritten = channelBotSetupRewrite.input({ url });
    expect(defaultParseSearch(rewritten.search)).toEqual({ field1: value });
  });

  it("leaves query parsing on other routes unchanged", () => {
    const url = new URL(
      "https://nyxid.example/admin/usage?page=2&enabled=true",
    );
    const original = url.href;
    expect(channelBotSetupRewrite.input({ url }).href).toBe(original);
    expect(defaultParseSearch(url.search)).toEqual({ page: 2, enabled: true });
  });
});
