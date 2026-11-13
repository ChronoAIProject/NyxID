import { describe, expect, it } from "vitest";
import { FEATURE_FLAG } from "@/lib/feature-flags";
import type { User } from "@/types/api";
import { accountPanelAllowed } from "./account-panel-availability";

function person(features: string[]): User {
  return {
    id: "person", email: "person@example.com", display_name: "Person", avatar_url: null,
    email_verified: true, mfa_enabled: false, is_admin: false, is_active: true,
    created_at: "2026-01-01T00:00:00Z",
    capabilities: { billing_available: true, enabled_features: features },
  };
}

describe("accountPanelAllowed", () => {
  it("opens the notifications feed only with the engine and the Notifications feature together", () => {
    expect(accountPanelAllowed("notifications", person([FEATURE_FLAG.NYXAGENT_ENGINE, FEATURE_FLAG.ASSISTANT_NOTIFICATIONS]))).toBe(true);
    expect(accountPanelAllowed("notifications", person([FEATURE_FLAG.NYXAGENT_ENGINE]))).toBe(false);
    expect(accountPanelAllowed("notifications", person([FEATURE_FLAG.ASSISTANT_NOTIFICATIONS]))).toBe(false);
    expect(accountPanelAllowed("notifications", person([]))).toBe(false);
  });

  it("keeps the other panels' rules", () => {
    expect(accountPanelAllowed("nyxbot", person([FEATURE_FLAG.NYXAGENT_ENGINE]))).toBe(true);
    expect(accountPanelAllowed("nyxbot", person([]))).toBe(false);
    expect(accountPanelAllowed("settings", person([]))).toBe(true);
    expect(accountPanelAllowed("billing", person([]))).toBe(true);
  });
});
