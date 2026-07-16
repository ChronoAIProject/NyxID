import { beforeEach, describe, expect, it, vi } from "vitest";
import type { UserCapabilities } from "@/types/api";
import { FEATURE_FLAG } from "./feature-flags";
import {
  fetchAssistantAccessUser,
  shouldRedirectFromAssistant,
} from "./assistant-availability";

const { mockGet } = vi.hoisted(() => ({
  mockGet: vi.fn(),
}));

vi.mock("@/lib/api-client", () => ({
  api: { get: mockGet },
}));

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

describe("fetchAssistantAccessUser", () => {
  beforeEach(() => {
    mockGet.mockReset();
  });

  it("returns the server-resolved user from /users/me", async () => {
    const user = userWithAssistant(true);
    mockGet.mockResolvedValueOnce(user);

    await expect(fetchAssistantAccessUser()).resolves.toBe(user);
    expect(mockGet).toHaveBeenCalledWith("/users/me");
  });

  it("fails closed to null when the verification fetch rejects", async () => {
    mockGet.mockRejectedValueOnce(new Error("network down"));

    await expect(fetchAssistantAccessUser()).resolves.toBeNull();
  });

  it("treats a null verified user as flag-off (guard redirects)", () => {
    expect(shouldRedirectFromAssistant({ isLoading: false, user: null })).toBe(
      true,
    );
  });
});
