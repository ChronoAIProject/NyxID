import { describe, expect, it } from "vitest";
import {
  getTelegramIdentity,
  normalizeTelegramBotUsername,
  parseTelegramCallbackSearch,
} from "./telegram-login";

describe("normalizeTelegramBotUsername", () => {
  it("strips a leading @ when present", () => {
    expect(normalizeTelegramBotUsername("@nyxid_bot")).toBe("nyxid_bot");
  });

  it("keeps plain bot usernames unchanged", () => {
    expect(normalizeTelegramBotUsername("nyxid_bot")).toBe("nyxid_bot");
  });
});

describe("parseTelegramCallbackSearch", () => {
  it("returns non-telegram result when widget params are absent", () => {
    const result = parseTelegramCallbackSearch(
      new URLSearchParams("status=success"),
    );

    expect(result.isTelegramCallback).toBe(false);
    expect(result.payload).toBeNull();
    expect(result.error).toBeNull();
  });

  it("parses a valid Telegram callback payload", () => {
    const result = parseTelegramCallbackSearch(
      new URLSearchParams(
        "provider_id=provider-1&id=42&first_name=Nyx&username=nyxid&auth_date=1700000000&hash=abc123",
      ),
    );

    expect(result.error).toBeNull();
    expect(result.payload).toEqual({
      providerId: "provider-1",
      data: {
        id: "42",
        first_name: "Nyx",
        last_name: undefined,
        username: "nyxid",
        photo_url: undefined,
        auth_date: 1700000000,
        hash: "abc123",
      },
    });
  });

  it("reports an error when provider_id is missing", () => {
    const result = parseTelegramCallbackSearch(
      new URLSearchParams(
        "id=42&first_name=Nyx&auth_date=1700000000&hash=abc123",
      ),
    );

    expect(result.isTelegramCallback).toBe(true);
    expect(result.payload).toBeNull();
    expect(result.error).toBe("Missing provider ID in Telegram callback.");
  });

  it("reports an error when auth_date is invalid", () => {
    const result = parseTelegramCallbackSearch(
      new URLSearchParams(
        "provider_id=provider-1&id=42&first_name=Nyx&auth_date=abc&hash=abc123",
      ),
    );

    expect(result.isTelegramCallback).toBe(true);
    expect(result.payload).toBeNull();
    expect(result.error).toBe("Invalid Telegram auth timestamp.");
  });
});

describe("getTelegramIdentity", () => {
  it("builds a display model from metadata", () => {
    const identity = getTelegramIdentity({
      telegram_user_id: 42,
      username: "nyxid",
      first_name: "Nyx",
      last_name: "Bot",
      photo_url: "https://example.com/avatar.png",
    });

    expect(identity).toEqual({
      userId: "42",
      username: "nyxid",
      firstName: "Nyx",
      lastName: "Bot",
      displayName: "Nyx Bot",
      subtitle: "@nyxid",
      photoUrl: "https://example.com/avatar.png",
    });
  });

  it("returns null when metadata has no Telegram identity fields", () => {
    expect(getTelegramIdentity(null)).toBeNull();
    expect(getTelegramIdentity({ unrelated: "value" })).toBeNull();
  });
});
