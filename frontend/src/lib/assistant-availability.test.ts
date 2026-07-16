import { describe, expect, it } from "vitest";
import type { UserCapabilities } from "@/types/api";
import { FEATURE_FLAG } from "./feature-flags";
import { shouldRedirectFromAssistant } from "./assistant-availability";

function userWithAssistant(enabled: boolean): {
  readonly capabilities: UserCapabilities;
} {
  return {
    capabilities: {
      enabled_features: enabled ? [FEATURE_FLAG.AI_ASSISTANT] : [],
    },
  };
}

describe("shouldRedirectFromAssistant", () => {
  it("waits while auth and capabilities are still loading", () => {
    expect(
      shouldRedirectFromAssistant({
        isLoading: true,
        user: null,
      }),
    ).toBe(false);
  });

  it("redirects when the feature is unavailable after auth settles", () => {
    expect(
      shouldRedirectFromAssistant({
        isLoading: false,
        user: userWithAssistant(false),
      }),
    ).toBe(true);
  });

  it("does not redirect when the feature is available", () => {
    expect(
      shouldRedirectFromAssistant({
        isLoading: false,
        user: userWithAssistant(true),
      }),
    ).toBe(false);
  });
});
