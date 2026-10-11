import { describe, expect, it } from "vitest";
import type { ChannelBotHealth } from "./channels";

describe("X channel bot health contract", () => {
  it("contains diagnostics, mode, and recovery action fields", () => {
    const health: ChannelBotHealth = {
      diagnostics: { last_inbound_source: "webhook", consecutive_errors: 2 },
      mode: "webhook",
      status: "failed",
      webhook_registered: false,
      poll_error_count: 0,
      recommended_action: "resubscribe",
      recommended_action_text: "Register the subscription",
    };
    expect(health.mode).toBe("webhook");
    expect(health.diagnostics.last_inbound_source).toBe("webhook");
    expect(health.recommended_action).toBe("resubscribe");
  });
});
