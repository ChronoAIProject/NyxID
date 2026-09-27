import { beforeEach, describe, expect, it } from "vitest";
import { transitionAssistantIdentity } from "@/lib/assistant/identity";
import {
  isCreditsDialogSuppressed,
  isInsufficientCreditsCode,
  isInsufficientCreditsHttp,
  mutationCreditsDenial,
  ownerCreditsPayer,
} from "./credits-denial";

describe("isInsufficientCreditsHttp", () => {
  it("recognizes the 402 symbol", () => {
    expect(
      isInsufficientCreditsHttp(402, {
        error: "insufficient_credits",
        error_code: 11300,
        message: "Insufficient credits",
      }),
    ).toBe(true);
  });

  it("accepts the numeric code only on a 402 without a symbol", () => {
    expect(isInsufficientCreditsHttp(402, { error_code: 11300 })).toBe(true);
    expect(
      isInsufficientCreditsHttp(402, { error: null, error_code: 11300 }),
    ).toBe(true);
  });

  it("never confuses ConnectLinkNotFound, which shares 11300", () => {
    expect(
      isInsufficientCreditsHttp(404, {
        error: "connect_link_not_found",
        error_code: 11300,
      }),
    ).toBe(false);
    expect(isInsufficientCreditsHttp(404, { error_code: 11300 })).toBe(false);
  });

  it.each([
    "wallet_suspended",
    "billing_not_configured",
    "billing_provider_unavailable",
    "plan_entitlement_required",
  ])("leaves other 402 variants (%s) to inline errors", (symbol) => {
    expect(
      isInsufficientCreditsHttp(402, { error: symbol, error_code: 11301 }),
    ).toBe(false);
  });

  it("lets an explicit different symbol win over the numeric code", () => {
    expect(
      isInsufficientCreditsHttp(402, {
        error: "wallet_suspended",
        error_code: 11300,
      }),
    ).toBe(false);
  });

  it("rejects non-object bodies", () => {
    expect(isInsufficientCreditsHttp(402, null)).toBe(false);
    expect(isInsufficientCreditsHttp(402, "insufficient_credits")).toBe(false);
  });
});

describe("isInsufficientCreditsCode", () => {
  it("matches only the exact symbol", () => {
    expect(isInsufficientCreditsCode("insufficient_credits")).toBe(true);
    for (const code of [
      11300,
      "11300",
      "nyxid_error",
      "assistant_unavailable",
      "http_402",
      null,
    ]) {
      expect(isInsufficientCreditsCode(code)).toBe(false);
    }
  });
});

describe("owner payers", () => {
  beforeEach(() => transitionAssistantIdentity("person-1"));

  it("maps the caller to self, anyone else to their org, and nothing to unknown", () => {
    expect(ownerCreditsPayer("person-1")).toBe("self");
    expect(ownerCreditsPayer("org-9")).toEqual({ org: { id: "org-9" } });
    expect(ownerCreditsPayer(undefined)).toBe("unknown");
  });

  it("keys one-shot mutations per attempt", () => {
    const first = mutationCreditsDenial("channel-bot-verify", "bot-1", "org-9");
    const second = mutationCreditsDenial(
      "channel-bot-verify",
      "bot-1",
      "org-9",
    );
    expect(first.key).toMatch(/^op:channel-bot-verify:bot-1:/);
    expect(first.key).not.toBe(second.key);
    expect(mutationCreditsDenial("send", "c", "person-1", "same").key).toBe(
      "op:send:c:same",
    );
  });
});

describe("isCreditsDialogSuppressed", () => {
  it.each([
    "/billing",
    "/login",
    "/register",
    "/oauth-consent",
    "/login/device",
    "/login/agent-key",
    "/login/code",
    "/connect/nyx_clk_abc",
    "/privacy",
    "/terms",
    "/",
  ])("suppresses %s", (path) => {
    expect(isCreditsDialogSuppressed(path)).toBe(true);
  });

  it.each([
    "/assistant",
    "/dashboard",
    "/channel-bots/connect/x",
    "/nyxbot/onboarding",
    "/billing/other",
  ])("presents on %s", (path) => {
    expect(isCreditsDialogSuppressed(path)).toBe(false);
  });
});
