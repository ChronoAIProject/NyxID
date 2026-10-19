import { defaultParseSearch } from "@tanstack/react-router";
import { describe, expect, it } from "vitest";
import { channelBotSetupRewrite, parseRawChannelBotReturnTo } from "./channel-bot-setup";

describe("parseRawChannelBotReturnTo", () => {
  const conversation = `nyxa-${"a".repeat(32)}`;
  const group = `nyxg-${"b".repeat(32)}`;

  it.each([
    [`?return_to=${conversation}`, conversation],
    [`return_to=${group}`, group],
  ])("accepts one canonical id: %s", (query, expected) => {
    expect(parseRawChannelBotReturnTo(query)).toBe(expected);
  });

  it.each([
    "",
    "?label=Support",
    "?return_to=",
    `?return_to=${conversation}&return_to=${conversation}`,
    `?return_to=${conversation}&return_to=${group}`,
    `?return_to=${conversation.toUpperCase()}`,
    `?return_to=${group.toUpperCase()}`,
    "?return_to=https://example.com/",
    "?return_to=//example.com",
    "?return_to=%2F%2Fexample.com",
    `?return_to=%6e${conversation.slice(1)}`,
    `?return%5Fto=${conversation}`,
    `?return_to=${JSON.stringify(conversation)}`,
    `?return_to=${conversation.slice(0, -1)}`,
    `?return_to=${group.slice(0, -1)}`,
    `?return_to=${conversation}junk`,
  ])("rejects unsafe or ambiguous values: %s", (query) => {
    expect(parseRawChannelBotReturnTo(query)).toBeUndefined();
  });
});

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
